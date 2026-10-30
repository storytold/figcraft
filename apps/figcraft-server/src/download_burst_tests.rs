//! A team library or plugin larger than one address's download burst (16 MiB) is served once the
//! address's budget is full, which it then owes: it comes more slowly, never not at all, and the
//! address still gets no more than its rate over time (#1089).

use std::time::{Duration, Instant};

use figcraft_collab::message::{Access, LibraryReply, LibraryReq};
use figcraft_collab_net::hub::Backend;
use figcraft_collab_net::net::Bucket;

use crate::abuse::{Abuse, AbuseConfig, Action, DOWNLOAD_BURST, DOWNLOAD_RATE};
use crate::hardening_tests::archive;
use crate::store::{BUSY_DOWNLOADS, Store};
use crate::test_util::{TempDir, config, ip, snapshot};

const MIB: usize = 1 << 20;

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

#[test]
fn a_bucket_lends_past_its_burst_only_when_full_and_is_paid_back_at_its_rate() {
    let mut b = Bucket::new(1.0, 4.0);
    let t = Instant::now();
    assert!(b.take_n(t, 1.0));
    assert!(!b.take_spread(t, 10.0), "not full: no loan");
    assert!(b.take_spread(t + secs(1), 10.0), "full again: lent, owing 6");
    assert!(!b.take_spread(t + secs(6), 1.0), "still owing 1");
    assert!(!b.take_spread(t + secs(10), 10.0), "3 of 4 back");
    assert!(b.take_spread(t + secs(11), 10.0), "paid back: lent again");
    assert!(b.take_spread(t + secs(21), 2.0), "within the burst as before");
}

#[test]
fn large_downloads_keep_an_address_to_its_rate() {
    let mut a = Abuse::new(AbuseConfig { open: false, connects_per_minute: 1, publishes_per_hour: 1, messages_per_second: 1 });
    let t = Instant::now();
    let me = ip("198.51.100.1");
    let big = 40 * MIB;
    // Asking for 40 MiB every second for ten minutes.
    let mut got = Vec::new();
    for s in 0..=600 {
        if a.allow(me, Action::Download(big), t + secs(s)) {
            got.push(s);
        }
    }
    let bytes = |within: u64| (got.iter().filter(|s| **s < within).count() * big) as f64;
    for minutes in 1..=10 {
        let most = big as f64 + DOWNLOAD_RATE * (minutes * 60) as f64;
        assert!(bytes(minutes * 60) <= most, "{} MiB in {minutes} min", bytes(minutes * 60) / MIB as f64);
    }
    assert!(bytes(601) >= DOWNLOAD_RATE * 600.0, "it is served at the rate, not refused: {got:?}");
    assert!(big as f64 > DOWNLOAD_BURST);
}

#[test]
fn a_library_larger_than_the_burst_is_downloaded_then_the_address_waits() {
    let dir = TempDir::new("biglib");
    let mut store = Store::open(dir.path(), config(dir.path()).store()).unwrap();
    let (file, ..) = store.publish("Library", &snapshot("Library")).unwrap();
    store.publish_library(&file.id, &archive(0, 40 * MIB)).unwrap();
    let fetch = |s: &mut Store, who: &str| s.library(ip(who), &file.id, Access::View, LibraryReq::Fetch { have: None, usage: None });
    assert!(matches!(fetch(&mut store, "198.51.100.1"), LibraryReply::Library { archive, .. } if archive.len() > 40 * MIB));
    assert!(matches!(fetch(&mut store, "198.51.100.1"), LibraryReply::Failed { reason } if reason == BUSY_DOWNLOADS), "then it waits");
    assert!(matches!(fetch(&mut store, "198.51.100.9"), LibraryReply::Library { .. }), "another address has its own share");
}
