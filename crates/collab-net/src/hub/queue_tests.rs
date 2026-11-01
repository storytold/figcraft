//! The hub's send queues (no writers: nothing is ever sent, so what is queued stays queued): a
//! broadcast counts once against the server's budget however many copies are queued, a block of
//! addresses can't take more than its share, and when the server's budget is used up the
//! connection holding the most is shed, not the next client that needs a little.

use std::collections::HashMap;
use std::net::{IpAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, sync_channel};

use tungstenite::Bytes;

use super::{BlockUse, Blocks, Charges, Conn, Kind, Line, Out, Push, deliver};

/// A line with its own queue counter, the given block counter and the server's.
fn line(block: &Arc<AtomicUsize>, all: &Arc<AtomicUsize>, max_block: usize, max_all: usize) -> (Line, Receiver<Out>) {
    let (tx, rx) = sync_channel(64);
    (Line { tx, queued: Arc::default(), block: block.clone(), all: all.clone(), max: 1 << 20, max_block, max_all }, rx)
}

fn socket() -> TcpStream {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    TcpStream::connect(l.local_addr().unwrap()).unwrap()
}

#[test]
fn a_broadcast_counts_once_against_the_server() {
    let all = Arc::new(AtomicUsize::new(0));
    let lines: Vec<_> = (0..4).map(|_| line(&Arc::default(), &all, usize::MAX, 10_000)).collect();
    let b = Bytes::from(vec![0u8; 3_000]);
    let mut held = Charges::default();
    for (l, _) in &lines {
        l.push(&b, &mut held, false).unwrap();
    }
    assert_eq!(all.load(Ordering::Relaxed), 3_000, "four copies of one message, counted once");
    assert!(lines.iter().all(|(l, _)| l.queued.load(Ordering::Relaxed) == 3_000), "each queue counts its copy");
    drop(held);
    // Sent (or dropped): the counts go back as the copies go.
    for (_, rx) in &lines {
        drop(rx.try_recv().unwrap());
    }
    assert_eq!(all.load(Ordering::Relaxed), 0);
    assert!(lines.iter().all(|(l, _)| l.queued.load(Ordering::Relaxed) == 0));
}

#[test]
fn one_block_gets_only_its_share() {
    let (block, all) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let (a, _ra) = line(&block, &all, 1_000, usize::MAX);
    let (b, _rb) = line(&block, &all, 1_000, usize::MAX);
    let (c, _rc) = line(&Arc::default(), &all, 1_000, usize::MAX);
    let msg = Bytes::from(vec![0u8; 800]);
    a.push(&msg, &mut Charges::default(), false).unwrap();
    assert_eq!(b.push(&msg, &mut Charges::default(), false), Err(Push::Full), "the block's second connection is over its share");
    c.push(&msg, &mut Charges::default(), false).unwrap();
}

#[test]
fn the_biggest_consumer_is_shed_when_the_budget_runs_out() {
    let all = Arc::new(AtomicUsize::new(0));
    let ip: IpAddr = "203.0.113.1".parse().unwrap();
    let mut conns = HashMap::new();
    let mut receivers = Vec::new();
    for id in 1..=3u64 {
        let (l, rx) = line(&Arc::default(), &all, usize::MAX, 10_000);
        receivers.push(rx);
        conns.insert(id, Conn { out: l, sock: socket(), kind: Kind::Managing { comments: true }, ip });
    }
    // Connection 1 doesn't read: it holds most of the budget.
    let big = Bytes::from(vec![0u8; 9_000]);
    assert!(deliver(&conns, 1, &big, &mut Charges::default(), &mut Vec::new()));
    // Connection 2 needs a little: the hog is shed and 2 still gets its message.
    let small = Bytes::from(vec![0u8; 2_000]);
    let mut shed = Vec::new();
    assert!(deliver(&conns, 2, &small, &mut Charges::default(), &mut shed));
    assert_eq!(shed, vec![1]);
    assert_eq!(conns[&2].out.queued.load(Ordering::Relaxed), 2_000);
    // When the receiver is itself the biggest, it is the one to go.
    let mut shed = Vec::new();
    assert!(!deliver(&conns, 1, &small, &mut Charges::default(), &mut shed), "the hog asking for more is dropped");
    assert!(shed.is_empty());
}

#[test]
fn a_broadcast_counts_once_per_block_and_welcomes_not_at_all() {
    let (office, all) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    // Twenty people behind one office's address, a block share of 3 MB.
    let lines: Vec<_> = (0..20).map(|_| line(&office, &all, 3_000_000, usize::MAX)).collect();
    // A 2 MB paste to all of them: one copy against the block, so everyone gets it.
    let paste = Bytes::from(vec![0u8; 2_000_000]);
    let mut charges = Charges::default();
    for (l, _) in &lines {
        l.push(&paste, &mut charges, false).unwrap();
    }
    assert_eq!(office.load(Ordering::Relaxed), 2_000_000);
    drop(charges);
    for (_, rx) in &lines {
        drop(rx.try_recv().unwrap());
    }
    assert_eq!(office.load(Ordering::Relaxed), 0);
    // Twenty welcomes of a large file at once (each its own message): not counted against the block.
    let welcome = Bytes::from(vec![0u8; 1_000_000]);
    for (l, _) in &lines {
        l.push(&welcome, &mut Charges { skip_blocks: true, ..Charges::default() }, false).unwrap();
    }
    assert_eq!(office.load(Ordering::Relaxed), 0);
    assert_eq!(all.load(Ordering::Relaxed), 20_000_000, "they still count against the server's budget");
}

#[test]
fn blocks_nobody_uses_are_forgotten() {
    let blocks = Blocks::default();
    let a: IpAddr = "203.0.113.1".parse().unwrap();
    let held: BlockUse = blocks.get(a);
    let _b = blocks.get("203.0.113.2".parse().unwrap());
    assert_eq!(blocks.len(), 2);
    drop(held);
    // The next connection prunes the block whose connections are all gone.
    let _c = blocks.get("203.0.113.3".parse().unwrap());
    assert_eq!(blocks.len(), 2, "203.0.113.1 went");
}
