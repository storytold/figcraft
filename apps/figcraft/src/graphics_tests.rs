//! The backends offered to wgpu: no Vulkan on Windows unless it is asked for (a faulting Vulkan
//! driver crashed the app as its window opened), and nothing changed anywhere else.

use eframe::wgpu::Backends;

use crate::graphics::backends;

/// What eframe offers when `WGPU_BACKEND` isn't set.
const DEFAULTS: Backends = Backends::PRIMARY.union(Backends::GL);

#[test]
fn windows_is_offered_direct3d_and_not_vulkan() {
    let offered = backends(None, true, DEFAULTS);
    assert!(offered.contains(Backends::DX12), "Direct3D 12 draws the window");
    assert!(offered.contains(Backends::GL), "OpenGL is there for hardware without a Direct3D 12 driver");
    assert!(!offered.contains(Backends::VULKAN), "a Vulkan driver that faults can't be picked");
}

#[test]
fn wgpu_backend_names_the_backend_on_every_system() {
    for windows in [true, false] {
        for asked in [Backends::VULKAN, Backends::DX12, Backends::GL, Backends::VULKAN | Backends::GL] {
            assert_eq!(backends(Some(asked), windows, DEFAULTS), asked, "windows: {windows}");
        }
    }
}

#[test]
fn other_systems_keep_eframes_defaults() {
    assert_eq!(backends(None, false, DEFAULTS), DEFAULTS);
    assert_eq!(backends(None, false, Backends::METAL), Backends::METAL);
}
