//! Which graphics API draws the window.
//!
//! eframe offers wgpu every backend it was built with, and between equal GPUs wgpu takes Vulkan
//! first, on Windows too. A Vulkan driver that faults there takes the app down as the window opens:
//! an access violation inside the vendor's driver, which no guard of ours can catch (seen on a laptop
//! with two AMD GPUs whose drivers were years apart). Direct3D 12 is part of Windows, so on Windows
//! wgpu is offered Direct3D 12, and OpenGL for hardware without a Direct3D 12 driver.
//!
//! `WGPU_BACKEND` (`dx12`, `vulkan`, `gl`, `metal`) still names the backend on every system, and
//! systems other than Windows keep eframe's defaults.

use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup};
use eframe::wgpu::Backends;

/// Direct3D 12, then OpenGL: wgpu ranks a hardware GPU on either ahead of Direct3D's software one.
const WINDOWS: Backends = Backends::DX12.union(Backends::GL);

/// The backends wgpu chooses from: the ones `WGPU_BACKEND` asks for (`requested`), else [`WINDOWS`]
/// on Windows, else eframe's `defaults`.
pub fn backends(requested: Option<Backends>, windows: bool, defaults: Backends) -> Backends {
    match requested {
        Some(requested) => requested,
        None if windows => WINDOWS,
        None => defaults,
    }
}

/// Offer wgpu [`backends`] instead of eframe's defaults.
pub fn configure(options: &mut WgpuConfiguration) {
    if let WgpuSetup::CreateNew(create) = &mut options.wgpu_setup {
        let offered = &mut create.instance_descriptor.backends;
        *offered = backends(Backends::from_env(), cfg!(windows), *offered);
    }
}
