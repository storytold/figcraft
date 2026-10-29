//! What the app asks a server outside a session, and what the answers mean, free of the
//! transport: the native one (`manage.rs`, `guest.rs`) and the browser's (`exchange.rs`) send the
//! same requests and read the answers the same way.

use std::sync::Arc;

use figcraft_collab::Replica;
use figcraft_collab::message::{Access, Admin, AdminReply, NewLink};
use figcraft_doc::Document;
use figcraft_engine::live::{Published, Shared, SharedLink};

use crate::invite::{Endpoint, Invite};

/// What publishing uploads, and where.
pub struct Publish {
    pub server: Endpoint,
    /// The server's key (`None`: publish as anyone may, on a server open to the public).
    pub key: Option<String>,
    /// The file's name on the server.
    pub name: String,
    pub doc: Arc<Document>,
    /// Share the file once it is there: links and join codes for others
    /// ([`Event::Shared`](figcraft_engine::live::Event::Shared)).
    pub share: bool,
}

/// How an error starts when the server said no (then its reason), rather than not answering.
pub(crate) const REFUSED: &str = "the server refused: ";

/// The server's key: the one given, else the address's `?key=…`; a blank one is none.
pub(crate) fn server_key(given: Option<String>, query: Vec<(String, String)>) -> Option<String> {
    given.or_else(|| query.into_iter().find(|(k, _)| k == "key").map(|(_, v)| v)).filter(|k| !k.trim().is_empty())
}

/// Why asking `server` about its hosting failed with `e`: a server older than open hosting can't
/// read the question.
pub(crate) fn hosting_error(server: &Endpoint, e: String) -> String {
    if e.contains("couldn't be read") {
        format!("{} takes files only with its key (it is older than this app): add ?key=… to the address", server.display())
    } else {
        e
    }
}

/// The request that keeps `doc` on a server as a new file named `name`.
pub(crate) fn upload(name: &str, doc: &Document) -> Result<Admin, String> {
    let snapshot = Replica::new(doc.clone()).snapshot().map_err(|e| e.to_string())?;
    Ok(Admin::Publish { name: name.to_string(), snapshot })
}

/// What `server`'s answer to [`upload`] means: the edit link to join the file with, and where it
/// is with its links (and its owner token, published without the key).
pub(crate) fn uploaded(server: &Endpoint, reply: Option<AdminReply>) -> Result<(Invite, Published), String> {
    let (file, edit, view, owner, expires) = match reply {
        Some(AdminReply::Published { file, edit, view }) => (file, edit, view, None, None),
        Some(AdminReply::Hosted { file, edit, view, owner, expires }) => (file, edit, view, Some(owner), expires),
        Some(AdminReply::Failed { reason }) => return Err(reason),
        _ => return Err("the server didn't say where the file is".into()),
    };
    let link = |token: String| Invite { server: server.clone(), file: file.id.clone(), token };
    let (edit, view) = (link(edit.token), link(view.token));
    let published = Published { server: server.display(), file: file.id, name: file.name, edit: edit.link(), view: view.link(), owner, expires };
    Ok((edit, published))
}

/// The links made for others: an edit and a view link, and a comment link from a server that
/// makes them.
pub(crate) struct Links {
    pub edit: NewLink,
    pub view: NewLink,
    pub comment: Option<NewLink>,
}

/// The request for a new link of `access` to `file`, for others.
fn link_request(file: &str, access: Access) -> Admin {
    Admin::CreateLink { file: file.to_string(), access, label: format!("Shared ({})", access.id()) }
}

/// The requests that revoke `file`'s links with the ids in `revoke`, then, to `create`, make a new
/// edit link and a new view link.
pub(crate) fn link_requests(file: &str, revoke: &[String], create: bool) -> Vec<Admin> {
    let mut requests: Vec<Admin> = revoke.iter().map(|id| Admin::Revoke { file: file.to_string(), link: id.clone() }).collect();
    requests.extend(create.then(|| [link_request(file, Access::Edit), link_request(file, Access::View)]).into_iter().flatten());
    requests
}

/// The request for a comment link to `file`, asked on a connection of its own once
/// [`made_links`] gave the others: a server from before comment links can't read it and closes
/// the connection (the file is shared without one then).
pub(crate) fn comment_request(file: &str) -> Vec<Admin> {
    vec![link_request(file, Access::Comment)]
}

/// The comment link the answers to [`comment_request`] made, if any.
pub(crate) fn made_comment(replies: Vec<AdminReply>) -> Option<NewLink> {
    replies.into_iter().find_map(|r| if let AdminReply::Link(l) = r { Some(l) } else { None })
}

/// The edit and view links the answers to [`link_requests`] made (`None`: none were asked for),
/// or why the server didn't do it; the comment link comes after ([`made_comment`]).
pub(crate) fn made_links(replies: Vec<AdminReply>, create: bool) -> Result<Option<Links>, String> {
    let mut made = Vec::new();
    for reply in replies {
        match reply {
            AdminReply::Link(l) => made.push(l),
            // A link that is gone already is as revoked as it gets.
            AdminReply::Failed { reason } if made.is_empty() && reason.contains("no such link") => {}
            AdminReply::Failed { reason } => return Err(reason),
            _ => {}
        }
    }
    match (made.pop(), made.pop()) {
        (Some(view), Some(edit)) => Ok(Some(Links { edit, view, comment: None })),
        _ if create => Err("the server didn't make the links".into()),
        _ => Ok(None),
    }
}

/// The requests for the join codes of the links [`made_links`] gave (asked on a connection of
/// their own: a server from before join codes can't read them and closes it).
pub(crate) fn code_requests(file: &str, links: &Links) -> Vec<Admin> {
    [&links.edit, &links.view].into_iter().chain(&links.comment).map(|l| Admin::Code { file: file.to_string(), token: l.token.clone() }).collect()
}

/// What others join `file` on `server` with: the links, with the codes `codes` (the answers to
/// [`code_requests`]; none from a server older than join codes).
pub(crate) fn shared(server: &Endpoint, file: &str, Links { edit, view, comment }: Links, codes: Vec<AdminReply>) -> Shared {
    let mut codes = codes.into_iter().map(|r| if let AdminReply::Code { code } = r { Some(code) } else { None });
    let mut link = |l: NewLink| SharedLink {
        link: Invite { server: server.clone(), file: file.to_string(), token: l.token }.link(),
        id: l.link.id,
        code: codes.next().flatten(),
    };
    let (edit, view) = (link(edit), Some(link(view)));
    Shared { edit, view, comment: comment.map(link) }
}

/// A hosting answer as the app shows it: a new link whole, in place of its token.
pub(crate) fn hosted(server: &Endpoint, reply: Option<AdminReply>) -> Option<AdminReply> {
    reply.map(|r| match r {
        AdminReply::Link(mut l) => {
            l.token = Invite { server: server.clone(), file: l.link.file.clone(), token: l.token }.link();
            AdminReply::Link(l)
        }
        other => other,
    })
}
