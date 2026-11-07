use super::limits::Admission;
use super::{Access, Admin, AdminReply, ClientMsg, HostInfo, IpAddr, LibraryReply, LibraryReq, PluginReply, PluginReq, Server};

/// An admin request's reply, and what the hub must do about connections it affects.
#[derive(Clone, Debug, PartialEq)]
pub struct Handled {
    pub reply: AdminReply,
    /// Links that were revoked: `(file, link)`; whoever joined with one is disconnected.
    pub revoked: Vec<(String, String)>,
    /// A file was removed: whoever is in it is disconnected and it is closed without saving.
    pub removed: Option<String>,
    /// An address was blocked ([`ip_key`]): its connections are dropped.
    pub blocked: Option<IpAddr>,
}

impl From<AdminReply> for Handled {
    fn from(reply: AdminReply) -> Self {
        Self { reply, revoked: Vec::new(), removed: None, blocked: None }
    }
}

/// What everyone is told when a hub stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Closing {
    /// The session is over (the app stopped hosting): final.
    Ended,
    /// The server is restarting: clients reconnect until it is back.
    Restarting,
}

/// What the hub knows of an open file (for listings).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpenFile {
    /// People connected to it.
    pub people: u32,
    /// Its latest sequence number.
    pub seq: u64,
}

/// Where a hub's files come from and what it keeps of them (see the module docs). The hub calls it
/// from its own thread only.
pub trait Backend: Send + 'static {
    /// A hello for `file` with `token`: what it may do, or why it may not join (the reason is
    /// shown to the person joining; never echo the token).
    fn admit(&mut self, file: &str, token: &str) -> Result<Admission, String>;

    /// The session for a file that isn't open (after [`admit`](Self::admit) let someone in).
    fn open(&mut self, file: &str) -> Result<Server, String>;

    /// The hub handled a message for an open file: keep what changed (its server's journal).
    fn changed(&mut self, _file: &str, _server: &mut Server) {}

    /// Make what [`changed`](Self::changed) kept durable; the hub sends the servers' answers
    /// after this. The files whose changes couldn't be made durable: nobody hears of them, the
    /// files are closed (the backend forgets their open state) and everyone in them reconnects.
    fn flush(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// Whether `file` has room for a batch of about `bytes` (its size on the wire): why not, and
    /// the batch is refused.
    fn room(&mut self, _file: &str, _server: &Server, _bytes: usize) -> Result<(), String> {
        Ok(())
    }

    /// Everyone left `file`: save it. True closes it (the next hello opens it again).
    fn idle(&mut self, _file: &str, _server: &mut Server) -> bool {
        false
    }

    /// The hub is stopping: save the open files.
    fn stop(&mut self, _open: Vec<(&str, &mut Server)>) {}

    /// A managing connection's key: whether it may manage files.
    fn manage(&mut self, _key: &str) -> Result<(), String> {
        Err("this session is hosted by an app: it has no files to manage".into())
    }

    /// A request on a managing connection from `ip` ([`ip_key`]); `open` tells about a file that
    /// is open.
    fn admin(&mut self, _ip: IpAddr, _req: Admin, _open: &dyn Fn(&str) -> Option<OpenFile>) -> Handled {
        AdminReply::Failed { reason: "nothing to manage here".into() }.into()
    }

    /// A request on a library connection to `file`, opened with a link of `access` (checked by
    /// [`admit`](Self::admit)).
    fn library(&mut self, _ip: IpAddr, _file: &str, _access: Access, _req: LibraryReq) -> LibraryReply {
        LibraryReply::Failed { reason: "this session is hosted by an app: team libraries live on a figcraft-server".into() }
    }

    /// A request on a plugin directory connection; `publisher`: it was opened with the right key
    /// (checked by [`manage`](Self::manage)).
    fn plugins(&mut self, _ip: IpAddr, _publisher: bool, _req: PluginReq) -> PluginReply {
        PluginReply::Failed { reason: "this session is hosted by an app: plugin directories live on a figcraft-server".into() }
    }

    /// A new connection from `ip` ([`ip_key`]): whether it may go on (why not is shown to the
    /// client).
    fn connect(&mut self, _ip: IpAddr) -> Result<(), String> {
        Ok(())
    }

    /// A message from a connection from `ip`: false disconnects it (that address sends too much).
    fn message(&mut self, _ip: IpAddr, _msg: &ClientMsg) -> bool {
        true
    }

    /// What to tell a client that reads it about the server's hosting, after its welcome to
    /// `file` (`None`: nothing).
    fn hosting(&mut self, _file: &str) -> Option<HostInfo> {
        None
    }

    /// An open hosting connection from `ip`, with a file's owner token when `file` isn't empty:
    /// what the server says about its hosting, or why the connection is refused.
    fn open_hosting(&mut self, _ip: IpAddr, _file: &str, _owner: &str) -> Result<HostInfo, String> {
        Err("this session is hosted by an app: it doesn't keep files for anyone".into())
    }

    /// A request on an open hosting connection from `ip`; `owns`: the file whose owner token it
    /// was opened with.
    fn hosted(&mut self, _ip: IpAddr, _owns: Option<&str>, _req: Admin, _open: &dyn Fn(&str) -> Option<OpenFile>) -> Handled {
        AdminReply::Failed { reason: "nothing to manage here".into() }.into()
    }

    /// Someone in `file` (joined with `link`, from `ip`) reports it to the operator.
    fn report(&mut self, _file: &str, _link: Option<&str>, _ip: IpAddr, _reason: &str) -> Result<(), String> {
        Err("this session is hosted by an app: tell the person hosting it".into())
    }

    /// A join code typed by someone at `ip`: the file it opens and that link's token, or why not
    /// (shown to them; wrong codes are counted against the address).
    fn resolve(&mut self, _ip: IpAddr, _code: &str) -> Result<(String, String), String> {
        Err("this session is hosted by an app: join it with its own code or invite link".into())
    }

    /// The token of the team link of `file` on the server's team list, asked from `ip`, or why not
    /// (shown to them).
    fn resolve_team(&mut self, _ip: IpAddr, _file: &str) -> Result<String, String> {
        Err("this session is hosted by an app: it has no team list".into())
    }

    /// What everyone is told when the hub stops.
    fn closing(&self) -> Closing;
}
