//! Requests to a server outside a session for the browser client, free of the browser so they run
//! (and are tested) on any target: an [`Exchange`] is one managing connection (its opening, then
//! requests answered in order, as `manage.rs` does on a blocking socket), and a [`Next`] says what
//! follows its answers: another exchange, or the events for the UI and the file to join. The
//! [`Driver`](crate::driver::Driver) runs them over its socket.
//!
//! Publishing ([`publish`]) uploads the document, makes the links and codes others join with when
//! it shares, then joins the new file with its edit link; [`background`] sends the hosting and
//! sharing requests the engine sends in the background (asking a server about its hosting,
//! starting and stopping a share).

use std::collections::VecDeque;

use figcraft_collab::message::{self, Admin, AdminReply, ClientMsg, HostInfo, Manage, OpenHosting, PROTOCOL, ServerMsg};
use figcraft_engine::live::{Event, NetReply, NetRequest, Shared, Status};

use crate::invite::{Endpoint, Invite};
use crate::requests::{self, Publish, REFUSED};

/// One managing connection to a server: what opens it, then requests answered in order.
pub(crate) struct Exchange {
    pub(crate) server: Endpoint,
    open: ClientMsg,
    requests: VecDeque<Admin>,
    answers: Answers,
    /// The server let the connection in (its first answer came).
    ready: bool,
    /// A request is out.
    asked: bool,
}

/// What an exchange got: the server's hosting (opened without the key) and the answers to the
/// requests, in order.
#[derive(Default)]
pub(crate) struct Answers {
    pub(crate) hosting: Option<HostInfo>,
    pub(crate) replies: Vec<AdminReply>,
}

/// What follows an exchange's answers (or why it failed).
pub(crate) type Then = Box<dyn FnOnce(Result<Answers, String>) -> Next>;

/// What comes next.
pub(crate) enum Next {
    /// Another exchange, on a connection of its own.
    Ask(Box<Exchange>, Then),
    /// Done: tell the UI these, then join the file this invite names (`None`: nothing to join).
    Done(Vec<Event>, Option<Invite>),
}

impl Exchange {
    /// Ask `server` `requests`: with its `key`, else (open hosting) as anyone, or as the owner of
    /// `file` with its `owner` token.
    pub(crate) fn new(server: Endpoint, key: Option<&str>, file: &str, owner: &str, requests: Vec<Admin>) -> Self {
        let open = match key {
            Some(key) => ClientMsg::Manage(Manage { protocol: PROTOCOL, key: key.to_string(), client_version: message::client_version() }),
            None => ClientMsg::OpenHosting(OpenHosting {
                protocol: PROTOCOL,
                client_version: message::client_version(),
                file: file.to_string(),
                owner: owner.to_string(),
            }),
        };
        Self { server, open, requests: requests.into(), answers: Answers::default(), ready: false, asked: false }
    }

    /// The connection's first message.
    pub(crate) fn opening(&self) -> &ClientMsg {
        &self.open
    }

    /// Every request is answered.
    pub(crate) fn done(&self) -> bool {
        self.ready && !self.asked && self.requests.is_empty()
    }

    pub(crate) fn answers(self) -> Answers {
        self.answers
    }

    /// Why the exchange failed with `e`, as the app says it: a server older than open hosting
    /// can't read the opening.
    pub(crate) fn failed(&self, e: String) -> String {
        if self.ready || !matches!(self.open, ClientMsg::OpenHosting(_)) { e } else { requests::hosting_error(&self.server, e) }
    }

    /// Take in one message from the server: the next request to send, if one is due; an error
    /// ends the exchange.
    pub(crate) fn take(&mut self, m: ServerMsg) -> Result<Option<ClientMsg>, String> {
        match m {
            ServerMsg::Refused { reason } => return Err(self.failed(format!("{REFUSED}{reason}"))),
            ServerMsg::Pong => return Ok(None),
            ServerMsg::Admin(r) if self.asked => self.answers.replies.push(*r),
            ServerMsg::Admin(r) if !self.ready && matches!(self.open, ClientMsg::Manage(_)) => match *r {
                AdminReply::Ready => self.ready = true,
                other => return Err(format!("the server answered {other:?} instead of letting this connection manage files")),
            },
            ServerMsg::Hosting(info) if !self.ready && matches!(self.open, ClientMsg::OpenHosting(_)) => {
                self.ready = true;
                self.answers.hosting = Some(*info);
            }
            _ => return Err("the server sent something other than an answer".into()),
        }
        self.asked = false;
        Ok(self.requests.pop_front().map(|r| {
            self.asked = true;
            ClientMsg::Admin(r)
        }))
    }
}

/// The UI's failure: why the file couldn't be published (it stays as it was).
fn not_published(e: &str) -> Next {
    Next::Done(vec![Event::Status(Status::Failed { reason: format!("Couldn't publish to the server: {e}") })], None)
}

/// Publish `job` (see the module docs): [`Event::Hosting`] (published without the key),
/// [`Event::Published`] and, to share, [`Event::Shared`], then the join.
pub(crate) fn publish(job: Publish) -> Next {
    let Publish { server, key, name, doc, share } = job;
    let upload = match requests::upload(&name, &doc) {
        Ok(r) => r,
        Err(e) => return not_published(&e),
    };
    let ex = Exchange::new(server.clone(), key.as_deref(), "", "", vec![upload]);
    Next::Ask(
        Box::new(ex),
        Box::new(move |answers| {
            let (hosting, uploaded) = match answers {
                Ok(a) => (a.hosting, requests::uploaded(&server, a.replies.into_iter().next())),
                Err(e) => (None, Err(e)),
            };
            let (edit, published) = match uploaded {
                Ok(done) => done,
                Err(e) => return not_published(&e),
            };
            let mut events: Vec<Event> = hosting.map(Event::Hosting).into_iter().collect();
            events.push(Event::Published(published.clone()));
            if !share {
                return Next::Done(events, Some(edit));
            }
            // The file's first links stay its owner's (this app joins with one): others get their
            // own, which stopping takes away again.
            let owner = published.owner.unwrap_or_default();
            let then: LinksThen = Box::new(move |made| {
                events.push(Event::Shared(made.and_then(|s| s.ok_or_else(|| "the server didn't make the links".to_string()))));
                Next::Done(events, Some(edit))
            });
            share_links(Sharing { server, key, file: published.file, owner, revoke: Vec::new(), create: true }, then)
        }),
    )
}

/// What [`share_links`] changes: who else gets into `file` (as `manage::share` does).
struct Sharing {
    server: Endpoint,
    key: Option<String>,
    file: String,
    owner: String,
    revoke: Vec<String>,
    create: bool,
}

/// What [`share_links`] hands the links to.
type LinksThen = Box<dyn FnOnce(Result<Option<Shared>, String>) -> Next>;

/// Revoke the links `s.revoke` names, then (to create) make new ones, then a comment link and their
/// codes, each on a connection of its own; `then` gets the links.
fn share_links(s: Sharing, then: LinksThen) -> Next {
    let ex = Exchange::new(s.server.clone(), s.key.as_deref(), &s.file, &s.owner, requests::link_requests(&s.file, &s.revoke, s.create));
    Next::Ask(
        Box::new(ex),
        Box::new(move |answers| match answers.and_then(|a| requests::made_links(a.replies, s.create)) {
            Ok(Some(links)) => comment_link(s, links, then),
            other => then(other.map(|_| None)),
        }),
    )
}

/// Ask for a comment link beside `links`, then for their codes.
fn comment_link(s: Sharing, mut links: requests::Links, then: LinksThen) -> Next {
    let ex = Exchange::new(s.server.clone(), s.key.as_deref(), &s.file, &s.owner, requests::comment_request(&s.file));
    Next::Ask(
        Box::new(ex),
        Box::new(move |made| {
            // A server from before comment links: the file is shared without one.
            let made = made.map_err(|e| log::info!("live session: no comment link from {}: {e}", s.server.display()));
            links.comment = made.ok().and_then(|a| requests::made_comment(a.replies));
            codes(s, links, then)
        }),
    )
}

/// Ask for the codes of `links`; `then` gets them with their codes.
fn codes(s: Sharing, links: requests::Links, then: LinksThen) -> Next {
    let ex = Exchange::new(s.server.clone(), s.key.as_deref(), &s.file, &s.owner, requests::code_requests(&s.file, &links));
    Next::Ask(
        Box::new(ex),
        Box::new(move |codes| {
            // A server from before join codes: the links are shared without them.
            let codes = codes.map_err(|e| log::info!("live session: no join codes from {}: {e}", s.server.display())).unwrap_or_default();
            then(Ok(Some(requests::shared(&s.server, &s.file, links, codes.replies))))
        }),
    )
}

/// Background request `req` (answered as [`Event::Net`] with `job`), for the requests the
/// browser sends: about a server's hosting, and changing who else gets into a file. `None` for
/// the others.
pub(crate) fn background(job: u64, req: NetRequest) -> Option<Next> {
    let answer = move |reply: NetReply| Next::Done(vec![Event::Net { job, reply: Box::new(reply) }], None);
    Some(match req {
        NetRequest::Hosting { server, file, owner, req } => match Endpoint::parse(&server) {
            Err(e) => answer(NetReply::Hosting(Err(e))),
            Ok((server, _)) => Next::Ask(
                Box::new(Exchange::new(server.clone(), None, &file, &owner, req.into_iter().collect())),
                Box::new(move |answers| {
                    let r = answers.and_then(|a| {
                        let info = a.hosting.ok_or("the server didn't say about its hosting")?;
                        Ok((info, requests::hosted(&server, a.replies.into_iter().next())))
                    });
                    answer(NetReply::Hosting(r))
                }),
            ),
        },
        NetRequest::Share { server, file, owner, revoke, create } => match Endpoint::parse(&server) {
            Err(e) => answer(NetReply::Share(Err(e))),
            // As the server's operator when the address carries its key, else as the file's owner.
            Ok((server, query)) => {
                let key = requests::server_key(None, query);
                share_links(Sharing { server, key, file, owner, revoke, create }, Box::new(move |r| answer(NetReply::Share(r))))
            }
        },
        NetRequest::Plugins { .. } | NetRequest::Library { .. } | NetRequest::Upload(_) => return None,
    })
}
