//! Confirmed saves: what the bridge ([`crate::host`]) knows about the files it handed a host page,
//! kept apart from the browser so every step is testable without one (like [`crate::hostmode`]).
//!
//! A host that confirms saves (`?save=managed`, [`crate::query::confirms_saves`]) gets each
//! Save and Save As of the document with an `id` and answers `saved` with that `id` once it stored
//! the file. Until then the document keeps its unsaved changes. Only one document save waits at a
//! time: a newer one replaces it, and an answer for any other `id` (an older save, a repeat, one
//! the host made up) is ignored. A save the host doesn't answer within the timeout counts as not
//! stored. A file the host opens forgets the save still waiting, so a late answer can't touch
//! the file opened since.
//!
//! The file the host opened, or the one the last Save As made, is kept here too: a Save As becomes
//! the document's file only once the host confirms it, so after a failed one the next Save is
//! still a Save As.

/// How long a document save waits for the host's answer before it counts as not stored.
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// A document save handed to the host and not answered yet.
#[derive(Debug)]
pub struct PendingSave<F> {
    /// The `id` the save carried; the host's `saved` names it.
    pub id: String,
    /// The file name it went out under.
    pub name: String,
    /// It went out as `saveAs: true`: confirmed, it becomes the document's file.
    pub save_as: bool,
    /// When it was handed over, in the caller's clock (milliseconds).
    pub started_at_ms: u64,
    /// The document as handed over. In the app this is its `FileSnapshot` (the tab's uid, its
    /// revision and contents), which marks the document saved as of the snapshot once confirmed.
    pub file: F,
}

/// A document save just handed to [`HostSaves::start`]: what to post with it.
#[derive(Debug, PartialEq, Eq)]
pub struct Started {
    pub id: String,
    pub save_as: bool,
}

/// The bridge's record of the document's file and the save waiting for the host.
#[derive(Debug)]
pub struct HostSaves<F> {
    host_file: Option<String>,
    /// The number in the next save's `id`. Never reset, so an `id` is never used twice on a page.
    next_id: u64,
    pending: Option<PendingSave<F>>,
    timeout_ms: u64,
}

impl<F> Default for HostSaves<F> {
    fn default() -> Self {
        Self::new(DEFAULT_TIMEOUT_MS)
    }
}

impl<F> HostSaves<F> {
    pub fn new(timeout_ms: u64) -> Self {
        Self { host_file: None, next_id: 1, pending: None, timeout_ms }
    }

    /// How long a save waits for its answer, in milliseconds.
    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }

    /// The file the host opened, or the one the last confirmed (or, without confirmation, posted)
    /// Save As made.
    pub fn host_file(&self) -> Option<&str> {
        self.host_file.as_deref()
    }

    /// Whether a file named `name` goes out as `saveAs: true` now ([`crate::hostmode::save_as`]).
    pub fn save_as(&self, name: &str) -> bool {
        crate::hostmode::save_as(name, self.host_file.as_deref())
    }

    /// A Save As went to a host that doesn't confirm saves: `name` is the document's file now.
    pub fn adopt(&mut self, name: &str) {
        self.host_file = Some(name.to_string());
    }

    /// The host sent the file `name` to open: it is the document's file now. A save still waiting
    /// was for the document open before, so it is forgotten and returned; its answer, if one
    /// comes, is ignored, and that document keeps its unsaved changes.
    pub fn opened(&mut self, name: &str) -> Option<PendingSave<F>> {
        self.host_file = Some(name.to_string());
        self.pending.take()
    }

    /// A Save or Save As of the document as `file`, under `name`, handed over at `now_ms`: the `id`
    /// and `saveAs` to post it with. A save still waiting is replaced, so its answer is ignored.
    pub fn start(&mut self, name: &str, file: F, now_ms: u64) -> Started {
        let id = format!("save-{}", self.next_id);
        self.next_id += 1;
        let save_as = self.save_as(name);
        self.pending = Some(PendingSave { id: id.clone(), name: name.to_string(), save_as, started_at_ms: now_ms, file });
        Started { id, save_as }
    }

    /// The host's `saved` for `id`: the save it confirms, when that is the one waiting. A Save As
    /// becomes the document's file now. Any other `id` changes nothing.
    pub fn saved(&mut self, id: &str) -> Option<PendingSave<F>> {
        let save = self.take(id)?;
        if save.save_as {
            self.host_file = Some(save.name.clone());
        }
        Some(save)
    }

    /// The save `id` could not be stored (the post failed, or the host refused it): the save,
    /// when that is the one waiting. The document's file stays as it was. Any other `id` changes
    /// nothing.
    pub fn failed(&mut self, id: &str) -> Option<PendingSave<F>> {
        self.take(id)
    }

    /// The save waiting since longer than the timeout at `now_ms`, which then counts as failed.
    pub fn expire(&mut self, now_ms: u64) -> Option<PendingSave<F>> {
        let waiting = self.pending.as_ref()?;
        if now_ms.saturating_sub(waiting.started_at_ms) < self.timeout_ms {
            return None;
        }
        self.pending.take()
    }

    /// How long from `now_ms` until the waiting save times out (so the app wakes then), or
    /// `None` when none waits.
    pub fn wait_ms(&self, now_ms: u64) -> Option<u64> {
        let waiting = self.pending.as_ref()?;
        Some((waiting.started_at_ms.saturating_add(self.timeout_ms)).saturating_sub(now_ms))
    }

    fn take(&mut self, id: &str) -> Option<PendingSave<F>> {
        if self.pending.as_ref().is_some_and(|p| p.id == id) { self.pending.take() } else { None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The document as handed over: in these tests, a label for the revision saved.
    type Saves = HostSaves<&'static str>;

    fn opened(name: &str) -> Saves {
        let mut saves = Saves::default();
        assert!(saves.opened(name).is_none());
        saves
    }

    #[test]
    fn each_save_gets_an_id_of_its_own() {
        let mut saves = opened("Poster.figcraft");
        let first = saves.start("Poster.figcraft", "r1", 0).id;
        let second = saves.start("Poster.figcraft", "r2", 0).id;
        assert_ne!(first, second);
        saves.opened("Other.figcraft");
        let third = saves.start("Other.figcraft", "r3", 0).id;
        assert!(third != first && third != second, "opening a file doesn't start the ids over");
    }

    #[test]
    fn a_confirmed_save_hands_back_what_was_saved() {
        let mut saves = opened("Poster.figcraft");
        let started = saves.start("Poster.figcraft", "r1", 5);
        assert!(!started.save_as, "a Save of the host's own file");
        let save = saves.saved(&started.id).expect("the waiting save");
        assert_eq!((save.file, save.name.as_str(), save.started_at_ms), ("r1", "Poster.figcraft", 5));
        assert!(saves.wait_ms(5).is_none(), "nothing waits any more");
        assert!(saves.saved(&started.id).is_none(), "a repeated answer changes nothing");
    }

    #[test]
    fn a_save_as_becomes_the_document_s_file_only_once_confirmed() {
        let mut saves = opened("Poster.figcraft");
        let started = saves.start("Poster 2.figcraft", "r1", 0);
        assert!(started.save_as);
        assert_eq!(saves.host_file(), Some("Poster.figcraft"), "not before the host stored it");
        saves.saved(&started.id);
        assert_eq!(saves.host_file(), Some("Poster 2.figcraft"));
        assert!(!saves.start("Poster 2.figcraft", "r2", 0).save_as, "the next Save is of that file");
    }

    #[test]
    fn a_failed_save_as_leaves_the_document_s_file_so_a_retry_is_still_a_save_as() {
        let mut saves = opened("Poster.figcraft");
        let started = saves.start("Poster 2.figcraft", "r1", 0);
        let save = saves.failed(&started.id).expect("the waiting save");
        assert_eq!(save.file, "r1");
        assert_eq!(saves.host_file(), Some("Poster.figcraft"));
        assert!(saves.start("Poster 2.figcraft", "r1", 0).save_as);
    }

    #[test]
    fn answers_for_other_ids_change_nothing() {
        let mut saves = opened("Poster.figcraft");
        let started = saves.start("Poster 2.figcraft", "r1", 0);
        for id in ["", "save-0", "save-99", "Save-1 ", "nonsense"] {
            assert!(saves.saved(id).is_none(), "{id:?}");
            assert!(saves.failed(id).is_none(), "{id:?}");
        }
        assert_eq!(saves.host_file(), Some("Poster.figcraft"));
        assert!(saves.saved(&started.id).is_some(), "the real answer still counts");
    }

    #[test]
    fn a_newer_save_replaces_the_one_waiting() {
        let mut saves = opened("Poster.figcraft");
        let older = saves.start("Poster 2.figcraft", "r1", 0);
        let newer = saves.start("Poster.figcraft", "r2", 10);
        assert!(saves.saved(&older.id).is_none(), "a late answer for the older save is ignored");
        assert!(saves.failed(&older.id).is_none());
        assert_eq!(saves.host_file(), Some("Poster.figcraft"), "the older Save As never became the file");
        assert_eq!(saves.saved(&newer.id).map(|s| s.file), Some("r2"));
    }

    #[test]
    fn a_save_left_unanswered_times_out() {
        let mut saves = Saves::new(1_000);
        saves.opened("Poster.figcraft");
        let started = saves.start("Poster 2.figcraft", "r1", 100);
        assert!(saves.expire(100).is_none());
        assert!(saves.expire(1_099).is_none(), "still inside the timeout");
        let remaining = saves.wait_ms(600).expect("a save waits");
        assert!(remaining > 0 && remaining < saves.timeout_ms(), "{remaining}");
        let save = saves.expire(1_100).expect("timed out");
        assert_eq!(save.file, "r1");
        assert_eq!(saves.host_file(), Some("Poster.figcraft"), "a timed-out Save As isn't the file");
        assert!(saves.saved(&started.id).is_none(), "an answer after the timeout is ignored");
        assert!(saves.expire(5_000).is_none() && saves.wait_ms(5_000).is_none());
    }

    #[test]
    fn a_clock_that_goes_back_does_not_end_the_wait_early() {
        let mut saves = Saves::new(1_000);
        saves.start("Untitled.figcraft", "r1", 500);
        assert!(saves.expire(0).is_none());
        assert!(saves.wait_ms(0).is_some_and(|w| w >= saves.timeout_ms()));
    }

    #[test]
    fn opening_a_file_forgets_the_save_still_waiting() {
        let mut saves = opened("Poster.figcraft");
        let started = saves.start("Poster 2.figcraft", "r1", 0);
        let dropped = saves.opened("Other.figcraft").expect("the save that waited");
        assert_eq!(dropped.file, "r1");
        assert!(saves.saved(&started.id).is_none(), "its late answer is ignored");
        assert_eq!(saves.host_file(), Some("Other.figcraft"), "and can't rename the file opened since");
        assert!(saves.expire(u64::MAX).is_none());
    }

    #[test]
    fn without_confirmation_a_save_as_is_the_document_s_file_once_posted() {
        let mut saves = Saves::default();
        assert!(saves.save_as("Untitled.figcraft"), "the host never sent a file");
        saves.adopt("Untitled.figcraft");
        assert!(!saves.save_as("Untitled.figcraft"));
        assert!(!saves.save_as("Untitled.png"), "an export is never the document's file");
    }
}
