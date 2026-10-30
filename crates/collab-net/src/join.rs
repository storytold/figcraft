//! What joining a session means on any transport, shared by the native guest ([`crate::guest`])
//! and the browser client: the join settings, how a connection ended and what follows, and taking
//! in what the host sends.

use std::sync::Mutex;
use std::time::Duration;

use figcraft_collab::Client;
use figcraft_collab::message::{ClientMsg, Peer, Resume, ServerMsg};
use figcraft_engine::live::{Event, MAX_PEERS, Role, SessionInfo, Status, TOO_LARGE_TO_JOIN};

use crate::inbox::Inbox;
use crate::invite::Invite;
use crate::limits::NetLimits;

/// What the host tells everyone when it stops (the WebSocket close reason).
pub const ENDED: &str = "The host ended the session";

/// What a server tells everyone when it stops (they reconnect once it is back).
pub const RESTARTING: &str = "The server is restarting";

/// WebSocket close codes hosts use: the app's session ended (going away), you were refused
/// (policy), or the server is restarting (service restart: try again).
pub(crate) const CLOSE_AWAY: u16 = 1001;
pub(crate) const CLOSE_POLICY: u16 = 1008;
pub(crate) const CLOSE_RESTART: u16 = 1012;

/// How a guest connects and reconnects.
#[derive(Clone, Debug)]
pub struct GuestConfig {
    pub invite: Invite,
    /// A join code to ask `invite.server` about first: its answer fills in the invite's file and
    /// token ([`resolved`]).
    pub code: Option<String>,
    /// Your name (empty: the host picks one).
    pub name: String,
    pub limits: NetLimits,
    pub connect_timeout: Duration,
    /// The pause before the first reconnect; it doubles each time up to `max_backoff`.
    pub backoff: Duration,
    pub max_backoff: Duration,
    /// Reconnect attempts before giving up.
    pub attempts: u32,
    /// Join as the client you were (an app that starts over after a crash): the first hello
    /// carries it, and a server that still knows the id gives it back.
    pub resume: Option<Resume>,
}

impl GuestConfig {
    pub fn new(invite: Invite, name: String) -> Self {
        Self {
            invite,
            code: None,
            name,
            limits: NetLimits::default(),
            connect_timeout: Duration::from_secs(5),
            backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(10),
            attempts: 12,
            resume: None,
        }
    }

    /// The pause before reconnect `attempt` (from 1).
    pub(crate) fn backoff(&self, attempt: u32) -> Duration {
        self.backoff.saturating_mul(1u32 << attempt.saturating_sub(1).min(16)).min(self.max_backoff)
    }
}

/// How a connection ended.
pub(crate) enum Outcome {
    /// Dropped: try again.
    Retry(String),
    /// The host said no (bad token, full…): final.
    Refused(String),
    /// The host ended the session: final.
    Ended(String),
    /// It can't go on, for this reason as it is shown: final.
    Failed(String),
    /// You left.
    Left,
}

/// The server couldn't be asked about a join code, or said no: final, in words for the person who
/// typed it (the server's own for a code it doesn't know).
pub(crate) fn unresolved(cfg: &GuestConfig, outcome: Outcome) -> Outcome {
    Outcome::Failed(match outcome {
        // A server from before join codes can't read the question.
        Outcome::Refused(why) if why.contains("couldn't be read") => {
            format!("{} doesn't know join codes yet (it runs an older version): ask for the invite link instead", cfg.invite.server.display())
        }
        Outcome::Refused(why) | Outcome::Failed(why) | Outcome::Ended(why) => why,
        Outcome::Retry(why) => format!("Couldn't reach the server ({why})"),
        Outcome::Left => return Outcome::Left,
    })
}

/// The host sent a message over this app's size limit (the file's welcome, usually): final, and
/// not the host's doing.
pub(crate) fn too_large(limits: &NetLimits) -> Outcome {
    Outcome::Failed(TOO_LARGE_TO_JOIN.replace("{mb}", &(limits.max_server_message >> 20).to_string()))
}

/// The server's answer to the join code: the invite is whole now.
pub(crate) fn resolved(cfg: &mut GuestConfig, file: String, token: String) -> Result<(), Outcome> {
    if !crate::invite::valid_id(&file, 1, 64) || !crate::invite::valid_id(&token, 16, 128) {
        return Err(Outcome::Failed("the server answered the join code with something that isn't a file".into()));
    }
    cfg.code = None;
    (cfg.invite.file, cfg.invite.token) = (file, token);
    Ok(())
}

/// What a close frame from the host means.
pub(crate) fn closed(code: u16, reason: &str) -> Outcome {
    match code {
        CLOSE_AWAY => Outcome::Ended(if reason.is_empty() { ENDED.to_string() } else { reason.to_string() }),
        CLOSE_POLICY => Outcome::Refused(reason.to_string()),
        CLOSE_RESTART => Outcome::Retry(if reason.is_empty() { RESTARTING.to_string() } else { reason.to_string() }),
        _ => Outcome::Retry("the host closed the connection".into()),
    }
}

/// What a guest does after a connection ended.
pub(crate) enum Next {
    /// Done: tell the UI this (nothing when you left).
    Stop(Option<Event>),
    /// Connect again after this pause (the UI has been told: `Reconnecting`).
    Reconnect(Duration),
}

/// After `outcome`: a refusal or the end of the session is final, and so is a first join that
/// can't reach the host; a live connection that dropped tries again with growing pauses until the
/// attempts run out.
pub(crate) fn after(outcome: Outcome, cfg: &GuestConfig, was_live: bool, attempt: &mut u32, inbox: &Inbox) -> Next {
    let failed = |reason: String| Next::Stop(Some(Event::Status(Status::Failed { reason })));
    match outcome {
        Outcome::Left => Next::Stop(None),
        Outcome::Refused(why) => failed(format!("The host refused: {why}")),
        Outcome::Failed(why) => failed(why),
        Outcome::Ended(reason) => Next::Stop(Some(Event::Ended { reason })),
        Outcome::Retry(why) if !was_live => failed(format!("Couldn't join the live session: {why}")),
        Outcome::Retry(why) => {
            *attempt += 1;
            if *attempt > cfg.attempts {
                return failed(format!("Lost the connection to the host ({why})"));
            }
            log::info!("live session: connection lost ({why}); reconnecting, attempt {attempt}");
            inbox.push(Event::Status(Status::Reconnecting { attempt: *attempt }));
            Next::Reconnect(cfg.backoff(*attempt))
        }
    }
}

/// Who this guest is in the session and the last sequence number it got (from the welcome and the
/// batches since): what a reconnect resumes with. The engine applies every message it got, in
/// order, before the next welcome, so its sync client is at that sequence number by then. (Shared
/// by the native guest's reader and writer threads.)
pub(crate) type Resuming = Mutex<Option<Resume>>;

/// The hello for a new connection: back after a drop, the same client catching up from where it
/// was.
pub(crate) fn hello(cfg: &GuestConfig, resume: &Resuming) -> ClientMsg {
    let mut hello = Client::hello(&cfg.invite.file, &cfg.invite.token, &cfg.name, "");
    if let (ClientMsg::Hello(h), Some(r)) = (&mut hello, resume.lock().ok().and_then(|r| *r)) {
        h.resume = Some(r);
    }
    hello
}

/// What a connection knows of the session (to tell the UI who's here).
#[derive(Default)]
pub(crate) struct Seen {
    info: Option<SessionInfo>,
    peers: Vec<Peer>,
}

/// What taking in one message from the host means for the connection.
pub(crate) enum Taken {
    Continue,
    /// The welcome: the connection is live (send the presence that waited).
    Welcomed,
    /// The connection should end.
    End(Outcome),
}

/// Take in one message from the host: events for the UI go to `inbox`, and `resume` follows the
/// session. On the welcome, `welcomed` runs before the UI hears of it: what the sync client sends
/// in answer must go out on this connection.
pub(crate) fn take(m: ServerMsg, cfg: &GuestConfig, inbox: &Inbox, seen: &mut Seen, resume: &Resuming, welcomed: &dyn Fn()) -> Taken {
    match m {
        ServerMsg::Welcome { session, client_id, seq, ref peers, access, secret, .. } => {
            if let Ok(mut r) = resume.lock() {
                *r = Some(Resume { session, client_id, seq, secret });
            }
            // The sync client opens the document (and fills in its name and pages).
            let peers: Vec<Peer> = peers.iter().take(MAX_PEERS).cloned().collect();
            let inv = &cfg.invite;
            let info = SessionInfo {
                role: Role::Guest,
                me: client_id,
                name: cfg.name.clone(),
                color: String::new(),
                file_name: String::new(),
                pages: Vec::new(),
                link: inv.link(),
                url: inv.url(),
                address: inv.server.display(),
                network: !inv.loopback(),
                access,
            };
            seen.peers = peers.clone();
            seen.info = Some(info.clone());
            welcomed();
            inbox.push(Event::Welcome { info, peers });
            inbox.push(Event::Sync(m));
            return Taken::Welcomed;
        }
        ServerMsg::PeerJoined(p) => match seen.info.as_mut() {
            // The host tells you how you are shown.
            Some(info) if p.client_id == info.me => {
                info.name = p.name;
                info.color = p.color;
                inbox.push(Event::Welcome { info: info.clone(), peers: seen.peers.clone() });
            }
            _ => {
                match seen.peers.iter().position(|q| q.client_id == p.client_id) {
                    Some(i) => {
                        seen.peers.remove(i);
                    }
                    None if seen.peers.len() >= MAX_PEERS => {
                        log::warn!("live session: {MAX_PEERS} people already here; leaving out another");
                        return Taken::Continue;
                    }
                    None => {}
                }
                seen.peers.push(p.clone());
                inbox.push(Event::PeerJoined(p));
            }
        },
        ServerMsg::PeerLeft { client_id } => {
            seen.peers.retain(|q| q.client_id != client_id);
            inbox.push(Event::PeerLeft(client_id));
        }
        ServerMsg::Presence { client_id, presence } => {
            if seen.info.as_ref().is_some_and(|i| i.me != client_id) {
                inbox.presence(client_id, presence);
            }
        }
        ServerMsg::Refused { reason } => return Taken::End(Outcome::Refused(reason)),
        ServerMsg::Changes(ref acc) => {
            if let Ok(mut r) = resume.lock()
                && let Some(r) = r.as_mut()
            {
                r.seq = acc.seq;
            }
            inbox.push(Event::Sync(m));
        }
        ServerMsg::Reject { .. } => inbox.push(Event::Sync(m)),
        // Checked again by the engine (the host is as untrusted as any peer).
        ServerMsg::Signal { client_id, signal } => inbox.push(Event::Signal(client_id, signal)),
        ServerMsg::Hosting(info) => inbox.push(Event::Hosting(*info)),
        ServerMsg::Reported { error } => inbox.push(Event::Reported(error)),
        // Answers for managing and library connections, and to a join code: not this one's.
        ServerMsg::Pong | ServerMsg::Admin(_) | ServerMsg::Library(_) | ServerMsg::Plugins(_) | ServerMsg::Resolved { .. } => {}
    }
    Taken::Continue
}
