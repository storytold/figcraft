//! Native video placement. Media is validated before the document acquires any asset.

#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

#[cfg(not(target_arch = "wasm32"))]
use base64::Engine as _;
#[cfg(not(target_arch = "wasm32"))]
use figcraft_color::{ImageScaleMode, Paint};
#[cfg(not(target_arch = "wasm32"))]
use figcraft_doc::{Node, NodeKind, VideoData};
use serde_json::{Value, json};

use super::CommandSpec;
use crate::{EngineError, Result, Session, spec};
#[cfg(not(target_arch = "wasm32"))]
use crate::{bad, params::*};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "video.place",
            "Place video",
            "{path | data (base64), name?, x?, y?, width?, height?, parent?, center?} → {id, video, poster, width, height, duration}. Silent Baseline H.264 MP4 only; at most 16 MiB, 1280×720 and 60 seconds.",
            true,
            place
        ),
        spec!("file.placeVideo", "Place video", "{path? | data?, x?, y?}; without input requests the video picker", true, place_dialog),
        spec!("proto.video.play", "Play prototype videos", "{} resumes videos in the active prototype player", false, |s, _| {
            s.request_ui("proto.video.play", json!({}));
            Ok(json!({"requested": true}))
        }),
        spec!("proto.video.pause", "Pause prototype videos", "{} pauses videos in the active prototype player", false, |s, _| {
            s.request_ui("proto.video.pause", json!({}));
            Ok(json!({"requested": true}))
        }),
        spec!("proto.video.restart", "Restart prototype videos", "{} restarts videos in the active prototype player", false, |s, _| {
            s.request_ui("proto.video.restart", json!({}));
            Ok(json!({"requested": true}))
        }),
    ]
}

#[cfg(not(target_arch = "wasm32"))]
fn input_bytes(p: &Value) -> Result<Vec<u8>> {
    let cap = figcraft_doc::MAX_VIDEO_BYTES;
    if let Some(data) = p.get("data").and_then(Value::as_str) {
        let data = data.split_once(',').map_or(data, |(_, body)| body).trim();
        if data.len() > cap.div_ceil(3) * 4 {
            return Err(bad("video.place", "video exceeds the 16 MiB input limit"));
        }
        let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| bad("video.place", format!("invalid base64: {e}")))?;
        if bytes.len() > cap {
            return Err(bad("video.place", "video exceeds the 16 MiB input limit"));
        }
        return Ok(bytes);
    }
    let path = req_string("video.place", p, "path")?;
    use std::io::Read;
    let file = std::fs::File::open(&path).map_err(|e| bad("video.place", e.to_string()))?;
    let mut bytes = Vec::new();
    file.take(cap as u64 + 1).read_to_end(&mut bytes).map_err(|e| bad("video.place", e.to_string()))?;
    if bytes.len() > cap {
        return Err(bad("video.place", "video exceeds the 16 MiB input limit"));
    }
    Ok(bytes)
}

fn place_dialog(s: &mut Session, p: &Value) -> Result<Value> {
    if p.get("path").is_some() || p.get("data").is_some() {
        return place(s, p);
    }
    s.request_ui("dialog.placeVideo", json!({}));
    Ok(json!({"requested": true}))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn place(s: &mut Session, p: &Value) -> Result<Value> {
    let page = s.page()?;
    let bytes = Arc::new(input_bytes(p)?);
    let hash = figcraft_format::image_hash(&bytes);
    let mut videos = s.doc()?.videos.clone();
    // Videos no layer paints any more would otherwise fill the store for good.
    let used = figcraft_doc::used_videos(s.doc()?);
    videos.retain(|key, _| used.contains(key));
    videos.insert(hash.clone(), VideoData { bytes: bytes.clone() });
    figcraft_doc::library::validate_videos(&videos).map_err(EngineError::Other)?;
    let mut source = figcraft_media::VideoSource::open(bytes).map_err(|e| bad("video.place", e.to_string()))?;
    let info = source.metadata().clone();
    let first = source.frame_at(0.0).map_err(|e| bad("video.place", e.to_string()))?;
    let pix = figcraft_render::from_rgba8(first.width, first.height, &first.rgba)
        .ok_or_else(|| bad("video.place", "decoder returned an invalid poster frame"))?;
    let poster_bytes = figcraft_render::encode_png(&pix).map_err(EngineError::Other)?;
    let scale = (1000.0 / f64::from(info.width.max(info.height))).min(1.0);
    let width = num(p, "width").unwrap_or(f64::from(info.width) * scale);
    let height = num(p, "height").unwrap_or(width * f64::from(info.height) / f64::from(info.width));
    if !(width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0 && width <= 1_000_000.0 && height <= 1_000_000.0) {
        return Err(bad("video.place", "width and height must be positive finite values at most 1000000"));
    }
    let (cx, cy) = s.ds().map(|ds| (ds.view.cx, ds.view.cy))?;
    let (mut x, mut y) = (num(p, "x").unwrap_or(cx), num(p, "y").unwrap_or(cy));
    if boolean(p, "center").unwrap_or(false) {
        x -= width / 2.0;
        y -= height / 2.0;
    }
    let parent = id(p, "parent").unwrap_or(page);
    let doc = s.doc_mut()?;
    let (poster, _, _) = figcraft_format::add_image(doc, poster_bytes).map_err(|e| bad("video.place", e.to_string()))?;
    doc.videos = videos;
    let nid = doc.alloc_id();
    let mut node = Node::new(nid, NodeKind::Rectangle, string(p, "name").unwrap_or_else(|| "Video".into()), width, height);
    node.transform = figcraft_geom::Affine::translate((x, y));
    node.fills = vec![Paint::video(hash.clone(), poster.clone(), ImageScaleMode::Fill)];
    doc.insert(node, parent, None)?;
    s.set_selection(vec![nid]);
    Ok(json!({"id": nid, "video": hash, "poster": poster, "width": width, "height": height, "duration": info.duration_seconds}))
}

#[cfg(target_arch = "wasm32")]
pub fn place(_s: &mut Session, _p: &Value) -> Result<Value> {
    Err(EngineError::Disabled(
        "video.place".into(),
        "Placing video requires the desktop app. Existing video documents show their posters in the browser.".into(),
    ))
}
