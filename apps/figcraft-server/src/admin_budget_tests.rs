//! Requests that change the server's files have a budget per address, with the server key or an
//! owner token too; join codes are found through an index of their hashes, with the same results
//! as comparing every link.

use std::net::IpAddr;

use figcraft_collab::message::{Admin, AdminReply};
use figcraft_collab_net::hub::Backend;

use crate::abuse::ADMIN_PER_MINUTE;
use crate::store::{BUSY_ADMIN, CODE_GONE, Store};
use crate::test_util::{TempDir, config, ip, snapshot};

fn rename(store: &mut Store, from: IpAddr, file: &str) -> AdminReply {
    Backend::admin(store, from, Admin::Rename { file: file.into(), name: "Renamed".into() }, &|_| None).reply
}

#[test]
fn a_flood_of_changes_from_one_address_is_held_back_and_others_are_not() {
    let dir = TempDir::new("admin-budget");
    let mut store = Store::open(dir.path(), config(dir.path()).store()).unwrap();
    let (file, ..) = store.publish("Poster", &snapshot("Poster")).unwrap();
    let flooder = ip("198.51.100.1");
    // The clock held still, so the budget doesn't refill while the requests run.
    store.frozen = Some(std::time::Instant::now());
    let done = (0..100).filter(|_| rename(&mut store, flooder, &file.id) == AdminReply::Done).count();
    assert_eq!(done, ADMIN_PER_MINUTE as usize, "a minute's worth, then wait");
    assert_eq!(rename(&mut store, flooder, &file.id), AdminReply::Failed { reason: BUSY_ADMIN.into() });
    // An owner token's requests share the address's budget.
    let hosted = store.hosted(flooder, Some(&file.id), Admin::Rename { file: file.id.clone(), name: "Mine".into() }, &|_| None).reply;
    assert_eq!(hosted, AdminReply::Failed { reason: BUSY_ADMIN.into() });
    // Looking costs nothing, and another address has its own budget.
    assert!(matches!(Backend::admin(&mut store, flooder, Admin::Files, &|_| None).reply, AdminReply::Files(_)));
    assert_eq!(rename(&mut store, ip("198.51.100.2"), &file.id), AdminReply::Done);
}

#[test]
fn codes_are_found_through_the_index_as_before() {
    let dir = TempDir::new("code-index");
    let files_dir = dir.path().join("files");
    let (files, codes) = {
        let mut store = Store::open(dir.path(), config(dir.path()).store()).unwrap();
        let mut files = Vec::new();
        let mut codes = Vec::new();
        for n in 0..12 {
            let (file, edit, _view) = store.publish(&format!("File {n}"), &snapshot("File")).unwrap();
            codes.push(store.set_code(&file.id, &edit.token).unwrap());
            files.push((file.id, edit.token, edit.link.id));
        }
        (files, codes)
    };
    // A copy of a file's folder holds the same code (the last file by id wins, as before).
    let copy = "zzzzzzzzzzzzzzzz";
    std::fs::create_dir_all(files_dir.join(copy)).unwrap();
    for f in std::fs::read_dir(files_dir.join(&files[0].0)).unwrap() {
        let f = f.unwrap();
        std::fs::copy(f.path(), files_dir.join(copy).join(f.file_name())).unwrap();
    }
    let mut store = Store::open(dir.path(), config(dir.path()).store()).unwrap();
    let who = |n: usize| ip(&format!("192.0.2.{n}"));
    for (n, ((id, token, _), code)) in files.iter().zip(&codes).enumerate().skip(1) {
        assert_eq!(store.resolve_code(who(n), code).unwrap(), (id.clone(), token.clone()), "file {n}");
    }
    assert_eq!(store.resolve_code(who(100), &codes[0]).unwrap().0, copy);
    // Revoking the copy's link leaves the original's code working; revoking that one ends it.
    store.revoke(copy, &files[0].2).unwrap();
    assert_eq!(store.resolve_code(who(101), &codes[0]).unwrap(), (files[0].0.clone(), files[0].1.clone()));
    store.revoke(&files[0].0, &files[0].2).unwrap();
    assert_eq!(store.resolve_code(who(102), &codes[0]).unwrap_err(), CODE_GONE);
    // A new code replaces the old in the index; a removed file's code is gone.
    let newer = store.set_code(&files[1].0, &files[1].1).unwrap();
    assert_eq!(store.resolve_code(who(103), &codes[1]).unwrap_err(), CODE_GONE);
    assert_eq!(store.resolve_code(who(104), &newer).unwrap().0, files[1].0);
    store.remove(&files[2].0).unwrap();
    assert_eq!(store.resolve_code(who(105), &codes[2]).unwrap_err(), CODE_GONE);
    assert_eq!(store.resolve_code(who(106), &codes[3]).unwrap().0, files[3].0);
}
