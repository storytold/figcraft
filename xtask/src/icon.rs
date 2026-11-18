//! The app icon: rasterises the master `assets/app-icon/figcraft.svg` (the chameleon, see
//! `assets/app-icon/README.md`) with the `resvg` CLI (`cargo install resvg`). Writes
//! figcraft-<size>.png (macOS grid), the hicolor theme and figcraft.ico. The outputs are
//! committed, so building and packaging never need `resvg`.

use std::path::Path;
use std::process::Command;

const VIEW_BOX: &str = r#"viewBox="0 0 512 512""#;
/// macOS: the 512-unit tile padded onto Apple's 824/1024 icon grid (transparent margin).
const MAC_VIEW_BOX: &str = r#"viewBox="-62 -62 636 636""#;

fn render(svg: &Path, size: u32, out: &Path) -> bool {
    let s = size.to_string();
    let ok = Command::new("resvg").args(["-w", &s, "-h", &s]).arg(svg).arg(out).status().is_ok_and(|st| st.success());
    if !ok {
        eprintln!("icon: resvg failed for {} (is it installed? `cargo install resvg`)", out.display());
    }
    ok
}

pub fn generate(root: &Path) -> bool {
    let dir = root.join("assets/app-icon");
    let Ok(master) = std::fs::read_to_string(dir.join("figcraft.svg")) else {
        eprintln!("icon: cannot read {}", dir.join("figcraft.svg").display());
        return false;
    };
    if !master.contains(VIEW_BOX) {
        eprintln!("icon: expected {VIEW_BOX} in figcraft.svg");
        return false;
    }
    let tmp = root.join("target/icon");
    // Windows and Linux show the tile edge to edge, rounded corners and all: the master as it is.
    let (mac, tile) = (tmp.join("mac.svg"), dir.join("figcraft.svg"));
    let mut ok = std::fs::create_dir_all(&tmp).is_ok() && std::fs::write(&mac, master.replacen(VIEW_BOX, MAC_VIEW_BOX, 1)).is_ok();
    if !ok {
        eprintln!("icon: cannot write {}", tmp.display());
        return false;
    }
    for size in [16u32, 32, 48, 64, 128, 256, 512, 1024] {
        ok &= render(&mac, size, &dir.join(format!("figcraft-{size}.png")));
        if size <= 512 {
            let hd = dir.join(format!("hicolor/{size}x{size}/apps"));
            let _ = std::fs::create_dir_all(&hd);
            ok &= render(&tile, size, &hd.join("ai.storyteller.figcraft.png"));
        }
    }
    // Windows .ico (PNG-compressed entries).
    let sizes = [16u32, 20, 24, 32, 40, 48, 64, 128, 256];
    let pngs: Vec<Vec<u8>> = sizes
        .iter()
        .filter_map(|s| {
            let png = tmp.join(format!("ico-{s}.png"));
            render(&tile, *s, &png).then(|| std::fs::read(png).ok()).flatten()
        })
        .collect();
    ok &= pngs.len() == sizes.len();
    let mut ico = vec![0, 0, 1, 0, pngs.len() as u8, 0];
    let mut offset = 6 + 16 * pngs.len() as u32;
    for (s, png) in sizes.iter().zip(&pngs) {
        let b = if *s >= 256 { 0 } else { *s as u8 };
        ico.extend_from_slice(&[b, b, 0, 0, 1, 0, 32, 0]);
        ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for png in &pngs {
        ico.extend_from_slice(png);
    }
    let ico_path = dir.join("figcraft.ico");
    let _ = std::fs::remove_file(&ico_path);
    ok &= std::fs::write(&ico_path, ico).is_ok();
    // The lighter trace keeps the scalable theme icon cheap to render.
    let sd = dir.join("hicolor/scalable/apps");
    let _ = std::fs::create_dir_all(&sd);
    let target = sd.join("ai.storyteller.figcraft.svg");
    let _ = std::fs::remove_file(&target);
    ok &= std::fs::copy(dir.join("figcraft-small.svg"), &target).is_ok();
    eprintln!("icon: wrote {}", dir.display());
    ok
}
