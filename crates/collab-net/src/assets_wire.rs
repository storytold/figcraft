//! Optional binary/lazy assets. Legacy peers still receive the original base64 Image entries;
//! a negotiated guest gets manifests and fetches immutable images separately, when needed.
use base64::Engine as _;
use figcraft_assets::{AssetRef, Blob, cache};
use figcraft_collab::change::{Change, Entry, ImageChange, Json};
use figcraft_collab::message::{self, Admin, ClientMsg};
use figcraft_collab::message::{CatchUp, ServerMsg};

#[cfg(not(target_arch = "wasm32"))]
pub(crate) const PROTOCOL: &str = "figcraft.assets.v1";

fn client_changes(msg: &mut ClientMsg, f: &mut impl FnMut(&mut Vec<Change>) -> Result<(), String>) -> Result<(), String> {
    match msg {
        ClientMsg::Changes { changes, .. } | ClientMsg::Admin(Admin::Publish { snapshot: changes, .. }) => f(changes),
        _ => Ok(()),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn server_changes(msg: &mut ServerMsg, f: &mut impl FnMut(&mut Vec<Change>) -> Result<(), String>) -> Result<(), String> {
    match msg {
        ServerMsg::Welcome { catch_up: CatchUp::Snapshot(changes), .. } | ServerMsg::Reject { fixes: changes, .. } => f(changes),
        ServerMsg::Welcome { catch_up: CatchUp::Delta(batches), .. } => {
            for batch in batches {
                f(&mut batch.changes)?;
            }
            Ok(())
        }
        ServerMsg::Changes(batch) => f(&mut batch.changes),
        _ => Ok(()),
    }
}

/// A peer without the resource extension cannot introduce local cache capabilities.
pub(crate) fn validate_legacy(msg: &ServerMsg) -> Result<(), String> {
    let inspect = |changes: &[Change]| {
        if changes.iter().any(|change| match change {
            Change::Image(_) => true,
            Change::Put { entry: Entry::Image(_), value } => value.0.get("asset").is_some(),
            _ => false,
        }) {
            return Err("the host sent image resources without negotiating the resource protocol".into());
        }
        Ok(())
    };
    match msg {
        ServerMsg::Welcome { catch_up: CatchUp::Snapshot(changes), .. } | ServerMsg::Reject { fixes: changes, .. } => inspect(changes),
        ServerMsg::Welcome { catch_up: CatchUp::Delta(batches), .. } => batches.iter().try_for_each(|batch| inspect(&batch.changes)),
        ServerMsg::Changes(batch) => inspect(&batch.changes),
        _ => Ok(()),
    }
}

fn image(change: &Change) -> Result<Option<ImageChange>, String> {
    match change {
        Change::Image(image) => Ok(Some(image.clone())),
        Change::Put { entry: Entry::Image(key), value } if value.0.get("asset").is_some() => {
            let mut image = value.read::<ImageChange>("image manifest").map_err(|e| e.to_string())?;
            image.key = key.clone();
            Ok(Some(image))
        }
        // Imported old journals and old peers can still supply the original representation.
        Change::Put { entry: Entry::Image(key), value } => {
            #[derive(serde::Deserialize)]
            struct Legacy {
                width: u32,
                height: u32,
                bytes: String,
            }
            let data = value.read::<Legacy>("image").map_err(|e| e.to_string())?;
            let bytes = base64::engine::general_purpose::STANDARD.decode(data.bytes).map_err(|e| e.to_string())?;
            let bytes = std::sync::Arc::new(bytes);
            Ok(Some(ImageChange {
                key: key.clone(),
                width: data.width,
                height: data.height,
                asset: AssetRef::of_arc(&bytes),
                bytes: Some(Blob(bytes)),
            }))
        }
        _ => Ok(None),
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Legacy,
    Binary,
    #[cfg(not(target_arch = "wasm32"))]
    Lazy,
}

fn prepare(changes: &mut [Change], mode: Mode) -> Result<(), String> {
    for change in changes {
        let Some(mut image) = image(change)? else { continue };
        if !image.asset.valid() {
            return Err("invalid image manifest".into());
        }
        // The replica retains current originals and bounded recent history. Lazy encoding
        // must not depend on disposable disk cache writes (or block every hosted room).
        match mode {
            #[cfg(not(target_arch = "wasm32"))]
            Mode::Lazy => {
                image.bytes = None;
                image.asset.scope.clear();
                *change = Change::Image(image);
            }
            Mode::Binary => {
                if image.bytes.is_none() {
                    image.bytes = Some(Blob(cache::require(&image.asset)?));
                }
                image.asset.scope.clear();
                *change = Change::Image(image);
            }
            Mode::Legacy => {
                let bytes = match &image.bytes {
                    Some(bytes) => bytes.0.clone(),
                    None => cache::require(&image.asset)?,
                };
                *change = Change::Put {
                    entry: Entry::Image(image.key),
                    value: Json(serde_json::json!({
                        "width": image.width, "height": image.height,
                        "bytes": base64::engine::general_purpose::STANDARD.encode(bytes.as_slice())
                    })),
                };
            }
        }
    }
    Ok(())
}

pub(crate) fn client(msg: &ClientMsg, binary: bool) -> Result<Vec<u8>, String> {
    let mut msg = msg.clone();
    if let ClientMsg::FetchAsset(asset) = &mut msg {
        asset.scope.clear();
    }
    client_changes(&mut msg, &mut |changes| prepare(changes, if binary { Mode::Binary } else { Mode::Legacy }))?;
    message::encode(&msg).map_err(|e| e.to_string())
}

/// Scopes on received manifests are local capabilities, derived from the joined origin/file.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn scope(msg: &mut ServerMsg, scope: &str) -> Result<(), String> {
    server_changes(msg, &mut |changes| {
        for change in changes {
            if let Some(mut image) = image(change)? {
                image.asset.scope = scope.to_string();
                *change = Change::Image(image);
            }
        }
        Ok(())
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn server(msg: &ServerMsg, lazy: bool) -> Result<Vec<u8>, String> {
    let mut msg = msg.clone();
    server_changes(&mut msg, &mut |changes| prepare(changes, if lazy { Mode::Lazy } else { Mode::Legacy }))?;
    message::encode(&msg).map_err(|e| e.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn manifests(msg: &ServerMsg) -> Result<Vec<AssetRef>, String> {
    let mut msg = msg.clone();
    let mut assets = std::collections::BTreeSet::new();
    server_changes(&mut msg, &mut |changes| {
        for change in changes {
            if let Some(image) = image(change)? {
                assets.insert(image.asset);
            }
        }
        Ok(())
    })?;
    Ok(assets.into_iter().collect())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn missing(msg: &ClientMsg) -> Result<Vec<AssetRef>, String> {
    let mut msg = msg.clone();
    let mut assets = std::collections::BTreeSet::new();
    client_changes(&mut msg, &mut |changes| {
        for change in changes {
            if let Some(image) = image(change)?
                && image.bytes.is_none()
                && cache::load(&image.asset).is_err()
            {
                assets.insert(image.asset);
            }
        }
        Ok(())
    })?;
    Ok(assets.into_iter().collect())
}

/// A local upload must be available under the same derived scope as its accepted manifest.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn remember_uploads(msg: &ClientMsg, scope: &str) -> Result<(), String> {
    let mut msg = msg.clone();
    client_changes(&mut msg, &mut |changes| {
        for change in changes {
            if let Some(mut image) = image(change)?
                && let Some(bytes) = image.bytes
            {
                cache::store_resident(&image.asset, &bytes.0)?;
                image.asset.scope = scope.to_string();
                cache::store_resident(&image.asset, &bytes.0)?;
            }
        }
        Ok(())
    })
}

/// A client may upload bytes it owns, but cannot attach a guessed cache digest to read another
/// file's cached image. Only the server emits references without their payloads.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn validate_upload(msg: &ClientMsg) -> Result<(), String> {
    let mut msg = msg.clone();
    client_changes(&mut msg, &mut |changes| {
        for change in changes {
            if let Some(image) = image(change)? {
                if !image.asset.scope.is_empty() {
                    return Err("image upload cannot supply a local cache scope".into());
                }
                if image.bytes.is_none() {
                    return Err("image upload is missing its binary payload".into());
                }
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn encoding_never_populates_the_source_cache_and_uploads_retain_the_session_scope() {
        let bytes = std::sync::Arc::new(format!("synthetic scoped upload {}", crate::token::file_id().unwrap()).into_bytes());
        let asset = AssetRef::of_arc(&bytes);
        let change = Change::Image(ImageChange {
            key: "synthetic-scoped".into(),
            width: 2,
            height: 2,
            asset: asset.clone(),
            bytes: Some(Blob(bytes.clone())),
        });
        assert!(cache::load(&asset).is_err());
        prepare(&mut [change.clone()], Mode::Legacy).unwrap();
        prepare(&mut [change.clone()], Mode::Binary).unwrap();
        prepare(&mut [change.clone()], Mode::Lazy).unwrap();
        assert!(cache::load(&asset).is_err(), "no encoder may write the disposable disk cache for resident images");
        let scope = AssetRef::of(b"synthetic outgoing session scope").sha256;
        let msg = ClientMsg::Changes { base_seq: 0, batch_id: 1, changes: vec![change.clone()] };
        remember_uploads(&msg, &scope).unwrap();
        let mut scoped = asset.clone();
        scoped.scope = scope;
        assert_eq!(cache::load(&scoped).unwrap(), bytes);
        assert_eq!(cache::load(&asset).unwrap(), bytes);
        prepare(&mut [change.clone()], Mode::Lazy).unwrap();
        prepare(&mut [change], Mode::Lazy).unwrap();
        assert_eq!(cache::load(&asset).unwrap(), bytes);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn foreign_upload_scope_is_refused_before_journaling() {
        let bytes = std::sync::Arc::new(b"synthetic scoped attack".to_vec());
        let mut asset = AssetRef::of_arc(&bytes);
        asset.scope = AssetRef::of(b"other file").sha256;
        let msg = ClientMsg::Changes {
            base_seq: 0,
            batch_id: 1,
            changes: vec![Change::Image(ImageChange { key: "img".into(), width: 1, height: 1, asset, bytes: Some(Blob(bytes)) })],
        };
        assert!(validate_upload(&msg).unwrap_err().contains("scope"));
        // The supported client strips this local capability, so normal edits still pass.
        let wire = client(&msg, true).unwrap();
        let normalized = message::decode(&wire, 1 << 20).unwrap();
        validate_upload(&normalized).unwrap();
    }

    #[test]
    fn binary_uploads_and_legacy_uploads_have_the_same_document_data() {
        let bytes = std::sync::Arc::new(vec![255; 100_000]);
        let msg = ClientMsg::Changes {
            base_seq: 0,
            batch_id: 1,
            changes: vec![Change::Image(ImageChange {
                key: "synthetic".into(),
                width: 20,
                height: 20,
                asset: AssetRef::of_arc(&bytes),
                bytes: Some(Blob(bytes)),
            })],
        };
        let modern = client(&msg, true).unwrap();
        let legacy = client(&msg, false).unwrap();
        assert!(modern.len() < 100_256);
        assert!(legacy.len() > modern.len() + 30_000);
        let modern: ClientMsg = message::decode(&modern, 200_000).unwrap();
        let legacy: ClientMsg = message::decode(&legacy, 200_000).unwrap();
        let ClientMsg::Changes { changes: a, .. } = modern else { panic!("no changes") };
        let ClientMsg::Changes { changes: b, .. } = legacy else { panic!("no changes") };
        assert_eq!(image(&a[0]).unwrap(), image(&b[0]).unwrap());
    }
}
