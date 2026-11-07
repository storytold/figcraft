use super::NodeId;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("command `{0}` is not available right now: {1}")]
    Disabled(String, String),
    #[error("invalid parameters for `{cmd}`: {msg}")]
    BadParams { cmd: String, msg: String },
    #[error("no open document")]
    NoDocument,
    #[error("no such node {0}")]
    NoNode(NodeId),
    #[error("{0}")]
    Other(String),
    /// A command run in the background sent a request to a server and runs again with the answer
    /// ([`background`]); never returned to other callers.
    #[error("waiting for the server")]
    Waiting,
    /// A bug: the command panicked. The document was rolled back.
    #[error("internal error in `{cmd}`: {msg} (the document was left as it was before; please report this bug)")]
    Internal { cmd: String, msg: String },
}

impl From<figcraft_doc::DocError> for EngineError {
    fn from(e: figcraft_doc::DocError) -> Self {
        match e {
            figcraft_doc::DocError::NoNode(id) => EngineError::NoNode(id),
            other => EngineError::Other(other.to_string()),
        }
    }
}

impl EngineError {
    /// This error from step `step` of a batch, saying so in its message; the kind stays (errors
    /// without a message stay as they are).
    pub(crate) fn at_step(self, step: usize) -> Self {
        match self {
            EngineError::BadParams { cmd, msg } => EngineError::BadParams { cmd, msg: format!("step {step}: {msg}") },
            EngineError::Other(msg) => EngineError::Other(format!("step {step}: {msg}")),
            e => e,
        }
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;

pub fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.to_string(), msg: msg.into() }
}
