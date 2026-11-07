//! Commands the UI runs without waiting on a server ([`Session::start_background`]): browsing and
//! installing from a plugin directory, publishing, adding and checking team libraries.
//!
//! Such a command runs as usual until it makes a server request ([`Session::net`]). Then the
//! request goes out on the link's own thread ([`crate::live::LiveLink::background`]) and the
//! command stops (its partial edit is rolled back like any failed command's). When the
//! answer arrives ([`Event::Net`](crate::live::Event::Net), taken by [`Session::live_poll`]) the
//! command runs again from the start, on the file it started on, and each request it makes is
//! answered from the answers so far, until it needs no new one: then its result is kept for the UI
//! ([`Session::take_background_done`]). A command's steps before its requests are cheap, so
//! running them again costs nothing worth noting.
//!
//! Every other caller (scripts, the CLI, the MCP server, the control channel) runs commands as
//! before, waiting for each answer; so does a link that can't send in the background. Nothing here
//! starts a request: only a command does, when something asks for it.

use serde_json::Value;

use crate::live::{NetReply, NetRequest};
use crate::{EngineError, Result, Session};

/// A command waiting for a server.
#[derive(Debug)]
struct Job {
    id: u64,
    command: String,
    params: Value,
    /// The file it started on (its uid), if one was open.
    doc: Option<u64>,
    /// The answers so far, with the requests they answer.
    answers: Vec<(NetRequest, NetReply)>,
    /// The request out now.
    out: Option<NetRequest>,
}

/// The run of a job in progress: the answers it may use (and which it used) and the request it
/// sent, if any.
#[derive(Debug)]
struct Run {
    job: u64,
    answers: Vec<(NetRequest, NetReply)>,
    used: Vec<bool>,
    out: Option<NetRequest>,
}

/// The commands waiting for servers, and the outcomes the UI hasn't taken yet.
#[derive(Debug, Default)]
pub struct Background {
    next: u64,
    jobs: Vec<Job>,
    run: Option<Run>,
    done: Vec<(String, Result<Value>)>,
}

impl Session {
    /// Run command `id` as [`execute`](Self::execute) does, but without waiting on a server (see
    /// the module docs): `Ok(Some(result))` when it finished at once, `Ok(None)` while its request
    /// is out (the outcome arrives through [`take_background_done`](Self::take_background_done)),
    /// or its error. A command already waiting is cancelled first: one job per command.
    pub fn start_background(&mut self, id: &str, params: Value) -> Result<Option<Value>> {
        self.cancel_background(id);
        self.background.next += 1;
        let job =
            Job { id: self.background.next, command: id.to_string(), params, doc: self.ds().ok().map(|d| d.uid), answers: Vec::new(), out: None };
        self.run_job(job)
    }

    /// Command `id` is waiting for a server.
    pub fn waiting_for(&self, id: &str) -> bool {
        self.background.jobs.iter().any(|j| j.command == id)
    }

    /// Stop waiting for command `id`: its answer is ignored when it arrives.
    pub fn cancel_background(&mut self, id: &str) {
        self.background.jobs.retain(|j| j.command != id);
    }

    /// The outcome of a command run in the background that finished since the last call: the
    /// command and its result.
    pub fn take_background_done(&mut self) -> Option<(String, Result<Value>)> {
        (!self.background.done.is_empty()).then(|| self.background.done.remove(0))
    }

    /// Run `job`'s command, answering its requests from its answers so far; it waits again when it
    /// sent a new one.
    fn run_job(&mut self, mut job: Job) -> Result<Option<Value>> {
        let shown = self.ds().ok().map(|d| d.uid);
        if let Some(uid) = job.doc {
            // A closed file leaves no document to act on.
            self.active = self.docs.iter().position(|d| d.uid == uid).unwrap_or(self.docs.len());
        }
        let used = vec![false; job.answers.len()];
        self.background.run = Some(Run { job: job.id, answers: std::mem::take(&mut job.answers), used, out: None });
        let result = self.execute(&job.command, &job.params);
        let run = self.background.run.take();
        if let Some(i) = shown.and_then(|uid| self.docs.iter().position(|d| d.uid == uid)) {
            self.active = i;
        }
        match run {
            Some(Run { answers, out: Some(out), .. }) => {
                job.answers = answers;
                job.out = Some(out);
                self.background.jobs.push(job);
                Ok(None)
            }
            _ => result.map(Some),
        }
    }

    /// The answer to background request `job` arrived: run its command again (an answer for a
    /// cancelled command is dropped).
    pub(crate) fn background_answer(&mut self, job: u64, reply: NetReply) {
        let Some(i) = self.background.jobs.iter().position(|j| j.id == job) else { return };
        let mut j = self.background.jobs.remove(i);
        let Some(sent) = j.out.take() else { return };
        j.answers.push((sent, reply));
        let command = j.command.clone();
        match self.run_job(j) {
            Ok(None) => {}
            Ok(Some(v)) => self.background.done.push((command, Ok(v))),
            Err(e) => self.background.done.push((command, Err(e))),
        }
    }

    /// One request to a server, for a command: answered from an earlier answer when the command
    /// runs again in the background, sent on the link's thread when it runs there for the first
    /// time ([`EngineError::Waiting`]), else waited for. With the answer, the request that was
    /// answered when it was an earlier one (what it carried then).
    pub(crate) fn net(&mut self, req: NetRequest) -> Result<(NetReply, Option<NetRequest>)> {
        let Some(link) = self.live.link() else { return Err(EngineError::Other(req.unavailable().into())) };
        if let Some(why) = crate::file_access::network_refused() {
            return Err(EngineError::Other(why));
        }
        if let Some(run) = self.background.run.as_mut() {
            let earlier = run.answers.iter().zip(run.used.iter_mut()).find(|((sent, _), used)| !**used && sent.stands_for(&req));
            if let Some(((sent, reply), used)) = earlier {
                *used = true;
                return Ok((reply.clone(), Some(sent.clone())));
            }
            // One request out at a time: anything after it waits for the next run.
            if run.out.is_some() {
                return Err(EngineError::Waiting);
            }
            if link.background(run.job, req.clone()) {
                run.out = Some(req);
                return Err(EngineError::Waiting);
            }
        }
        Ok((link.request(req), None))
    }
}

#[cfg(test)]
mod tests;
