//! The graphics attempts: no Vulkan on Windows at first (a faulting Vulkan driver crashed the app
//! as its window opened), another backend and then lower requirements when the graphics fail
//! before the first frame (#965, #1438), and never more starts than attempts.

use std::time::Duration;

use eframe::egui_wgpu::{WgpuConfiguration, WgpuError, WgpuSetup, WgpuSetupCreateNew};
use eframe::wgpu::{Backends, Dx12Compiler, InstanceFlags, PowerPreference};

use crate::control_server::bind;
use crate::graphics::{ATTEMPT_ENV, Attempt, Plan, STARTUP_FRAMES, apply, attempts, current, names, next, port_wait};

/// What eframe offers when `WGPU_BACKEND` isn't set.
const DEFAULTS: Backends = Backends::PRIMARY.union(Backends::GL);

const DX12_GL: Backends = Backends::DX12.union(Backends::GL);

const fn normal(backends: Backends) -> Attempt {
    Attempt { backends, lower: false }
}

const fn lower(backends: Backends) -> Attempt {
    Attempt { backends, lower: true }
}

#[test]
fn windows_starts_on_direct3d_then_vulkan_then_opengl_then_lower_requirements() {
    let list = attempts(None, true, false, DEFAULTS);
    assert_eq!(list, [normal(DX12_GL), normal(Backends::VULKAN), normal(Backends::GL), lower(DX12_GL)]);
    assert!(!list[0].backends.contains(Backends::VULKAN), "a Vulkan driver that faults can't be picked at first");
}

#[test]
fn linux_falls_back_to_opengl_then_lower_requirements() {
    assert_eq!(attempts(None, false, false, DEFAULTS), [normal(DEFAULTS), normal(Backends::GL), lower(DEFAULTS)]);
    // OpenGL alone is tried once.
    assert_eq!(attempts(None, false, false, Backends::GL), [normal(Backends::GL), lower(Backends::GL)]);
}

#[test]
fn macos_has_no_opengl_so_it_only_lowers_the_requirements() {
    assert_eq!(attempts(None, false, true, DEFAULTS), [normal(DEFAULTS), lower(DEFAULTS)]);
    assert_eq!(attempts(None, false, true, Backends::METAL), [normal(Backends::METAL), lower(Backends::METAL)]);
}

#[test]
fn wgpu_backend_names_the_one_attempt_on_every_system() {
    for (windows, macos) in [(true, false), (false, false), (false, true)] {
        for asked in [Backends::VULKAN, Backends::DX12, Backends::GL, Backends::VULKAN | Backends::GL] {
            assert_eq!(attempts(Some(asked), windows, macos, DEFAULTS), [normal(asked)], "windows: {windows}, macos: {macos}");
        }
    }
}

#[test]
fn the_attempt_comes_from_the_restart_and_stays_in_range() {
    assert_eq!(current(None, 4), 0);
    assert_eq!(current(Some("2"), 4), 2);
    assert_eq!(current(Some(" 1 "), 4), 1);
    assert_eq!(current(Some("9"), 4), 3, "past the end is the last");
    assert_eq!(current(Some("-1"), 4), 0);
    assert_eq!(current(Some("junk"), 4), 0);
    assert_eq!(current(Some("1"), 0), 0);
}

#[test]
fn only_a_graphics_failure_while_starting_up_starts_again_and_never_past_the_last_attempt() {
    assert_eq!(next(0, 4, true, 0), Some(1));
    assert_eq!(next(2, 4, true, STARTUP_FRAMES - 1), Some(3));
    assert_eq!(next(3, 4, true, 0), None, "the last attempt ends it");
    assert_eq!(next(0, 1, true, 0), None, "WGPU_BACKEND's one attempt ends it");
    assert_eq!(next(0, 4, false, 0), None, "not the graphics");
    assert_eq!(next(0, 4, true, STARTUP_FRAMES), None, "the window ran");
    // Following the restarts from the first start ends within the list.
    for count in 1..6 {
        let starts = std::iter::successors(Some(0), |&a| next(a, count, true, 0)).count();
        assert_eq!(starts, count);
    }
}

#[test]
fn the_message_names_the_apis_this_build_has() {
    let windows = attempts(None, true, false, DEFAULTS);
    assert_eq!(names(&windows, Backends::all()), "Direct3D 12, Vulkan, OpenGL");
    let linux = attempts(None, false, false, DEFAULTS);
    assert_eq!(names(&linux, Backends::VULKAN | Backends::GL), "Vulkan, OpenGL");
    let macos = attempts(None, false, true, DEFAULTS);
    assert_eq!(names(&macos, Backends::METAL), "Metal");
}

fn create_new(options: &WgpuConfiguration) -> Option<&WgpuSetupCreateNew> {
    match &options.wgpu_setup {
        WgpuSetup::CreateNew(create) => Some(create),
        WgpuSetup::Existing(_) => None,
    }
}

#[test]
fn every_attempt_compiles_with_fxc_and_leaves_the_indirect_validation_out() {
    for attempt in [normal(Backends::DX12), lower(Backends::METAL)] {
        let mut options = WgpuConfiguration::default();
        let power = create_new(&options).map(|c| c.power_preference);
        apply(&mut options, attempt);
        let create = create_new(&options).expect("eframe creates the instance");
        assert_eq!(create.instance_descriptor.backends, attempt.backends);
        if std::env::var_os("WGPU_DX12_COMPILER").is_none() {
            assert!(matches!(create.instance_descriptor.backend_options.dx12.shader_compiler, Dx12Compiler::Fxc));
        }
        if std::env::var_os("WGPU_VALIDATION_INDIRECT_CALL").is_none() {
            assert!(!create.instance_descriptor.flags.contains(InstanceFlags::VALIDATION_INDIRECT_CALL));
        }
        if std::env::var_os("WGPU_POWER_PREF").is_none() {
            let want = if attempt.lower { Some(PowerPreference::LowPower) } else { power };
            assert_eq!(Some(create.power_preference), want);
        }
    }
}

#[test]
fn a_start_again_waits_for_the_control_port_the_ending_app_still_holds() {
    let held = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a free port");
    let port = held.local_addr().expect("its address").port();
    let busy = bind(port, Duration::ZERO).expect_err("the port is held");
    assert_eq!(busy.kind(), std::io::ErrorKind::AddrInUse);
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        drop(held);
    });
    assert!(bind(port, Duration::from_secs(10)).is_ok(), "bound once the other app let go");
    release.join().expect("released");
    if std::env::var_os(ATTEMPT_ENV).is_none() {
        assert_eq!(port_wait(), Duration::ZERO, "a first start doesn't wait");
    }
}

#[test]
fn a_failure_that_starts_nothing_again_ends_with_the_error() {
    if std::env::var_os(ATTEMPT_ENV).is_some() {
        return;
    }
    let mut options = WgpuConfiguration::default();
    let plan = Plan::configure(&mut options);
    assert!(plan.finish(Ok(()), 0, true).is_ok());
    // A failure that isn't the graphics', and a graphics failure once the window ran: no restart.
    assert!(plan.finish(Err(eframe::Error::AppCreation("no app".into())), 0, true).is_err());
    let graphics = eframe::Error::Wgpu(WgpuError::NoSurfaceFormatsAvailable);
    assert!(plan.finish(Err(graphics), STARTUP_FRAMES, true).is_err());
}
