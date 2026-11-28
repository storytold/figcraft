//! Room kept ahead in a web page's heap while a large file is read (#2041).
//!
//! The allocator Rust links on `wasm32-unknown-unknown` (dlmalloc) asks the page for more memory
//! one 64 KiB page at a time whenever it runs out, and the browser changes the protection of the
//! whole memory at every grow. Joining a 121k-layer file grew the heap by about 1.5 GB in about
//! 20,000 grows; their total cost rises with the square of the heap size and took minutes on
//! Windows. Memory the allocator got is reused once freed and never handed back, so allocating one
//! large block and freeing it at once leaves that much room for what follows, for one grow.
//! [`keep_ahead`] does that whenever the memory grew since its last call: half the heap again, at
//! most [`MAX_STEP`], so the heap grows in a few dozen steps.
//!
//! It acts only while a snapshot is read ([`reading`]): the hooks sit in code every live edit runs
//! too (applying a batch, reading a property value), and an edit never reserves memory. Off the web
//! it does nothing. Between the steps of a read it only reads the memory size unless the memory
//! grew.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// A WebAssembly memory page.
const PAGE: usize = 64 * 1024;

/// The most room reserved at once: past a heap of twice this, the heap grows by this much a step
/// rather than by half, so a page close to the browser's limit doesn't ask for more than it needs.
const MAX_STEP: usize = 256 << 20;

/// How many bytes of room to reserve when the memory is `now` pages and was `seen` pages at the
/// last look (0: never looked, so only the size is noted).
fn room(seen: usize, now: usize) -> Option<usize> {
    (seen != 0 && now > seen).then(|| (now.saturating_mul(PAGE) / 2).min(MAX_STEP))
}

/// How many snapshot reads are under way (a full snapshot is applied inside a welcome's read).
static READING: AtomicUsize = AtomicUsize::new(0);

/// The memory size at the last look, in pages.
static SEEN: AtomicUsize = AtomicUsize::new(0);

/// Set when a reservation fails near the browser's memory limit so the rest of this read does not
/// retry failing large reservations on every subsequent growth.
static EXHAUSTED: AtomicBool = AtomicBool::new(false);

/// A snapshot read under way: [`keep_ahead`] acts until it is dropped, on error paths too.
pub(crate) struct Reading(());

impl Drop for Reading {
    fn drop(&mut self) {
        READING.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Start a snapshot read: growth from here on gets room kept ahead.
pub(crate) fn reading() -> Reading {
    if READING.fetch_add(1, Ordering::Relaxed) == 0 {
        EXHAUSTED.store(false, Ordering::Relaxed);
        SEEN.store(memory_pages().unwrap_or(0), Ordering::Relaxed);
    }
    Reading(())
}

/// Reserve room ahead in the heap if a snapshot is being read and the memory grew since the last
/// call (see the module docs).
pub(crate) fn keep_ahead() {
    if READING.load(Ordering::Relaxed) == 0 || EXHAUSTED.load(Ordering::Relaxed) {
        return;
    }
    let Some(now) = memory_pages() else { return };
    if let Some(bytes) = room(SEEN.load(Ordering::Relaxed), now) {
        let mut block: Vec<u8> = Vec::new();
        // Near the browser's memory limit this fails and nothing changes: stop trying large
        // reservations for the rest of this read and let the heap grow as needed.
        if block.try_reserve_exact(bytes).is_ok() {
            // Opaque to the optimizer, which could otherwise drop an allocation nothing reads.
            drop(std::hint::black_box(block));
        } else {
            EXHAUSTED.store(true, Ordering::Relaxed);
        }
    }
    SEEN.store(memory_pages().unwrap_or(now), Ordering::Relaxed);
}

/// The size of the page's memory in pages (`None` off the web: the system allocator there maps
/// large blocks on its own).
fn memory_pages() -> Option<usize> {
    #[cfg(target_arch = "wasm32")]
    {
        Some(core::arch::wasm32::memory_size(0))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{PAGE, room};

    /// A heap that grows by whole pages when an allocation doesn't fit, as the web allocator's.
    struct Heap {
        pages: usize,
        free: usize,
        grows: usize,
        seen: usize,
    }

    impl Heap {
        fn alloc(&mut self, n: usize) {
            if self.free < n {
                let pages = (n - self.free).div_ceil(PAGE);
                self.pages += pages;
                self.free += pages * PAGE;
                self.grows += 1;
            }
            self.free -= n;
        }

        fn keep_ahead(&mut self) {
            if let Some(bytes) = room(self.seen, self.pages) {
                self.alloc(bytes);
                self.free += bytes;
            }
            self.seen = self.pages;
        }
    }

    /// Reading a file the size of #2041's (46 MB to 1.66 GB in small allocations) grows the heap
    /// in a few dozen steps with room kept ahead between allocations, not in one step per page.
    #[test]
    fn room_kept_ahead_turns_page_by_page_growth_into_a_few_steps() {
        let run = |ahead: bool| {
            let mut h = Heap { pages: (46 << 20) / PAGE, free: 0, grows: 0, seen: 0 };
            while h.pages * PAGE < 1660 << 20 {
                h.alloc(400);
                if ahead {
                    h.keep_ahead();
                }
            }
            h.grows
        };
        let (plain, ahead) = (run(false), run(true));
        assert!(plain > 20_000, "page by page: {plain} grows");
        assert!(ahead * 200 < plain, "{ahead} grows with room kept ahead, {plain} without");
    }

    /// The first look only notes the size, and an unchanged or smaller memory reserves nothing.
    #[test]
    fn room_is_reserved_only_after_the_memory_grew() {
        assert_eq!(room(0, 1000), None);
        assert_eq!(room(1000, 1000), None);
        assert!(room(1000, 1001).is_some());
        assert!(room(1000, 1001) <= room(5000, 5001), "more room for a larger heap");
    }
}
