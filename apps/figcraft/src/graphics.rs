//! Which graphics API draws the window, and what happens when it can't.
//!
//! eframe offers wgpu every backend it was built with, and between equal GPUs wgpu takes Vulkan
//! first, on Windows too. A Vulkan driver that faults there takes the app down as the window opens:
//! an access violation inside the vendor's driver, which no guard of ours can catch (seen on a laptop
//! with two AMD GPUs whose drivers were years apart). Direct3D 12 is part of Windows, so on Windows
//! wgpu is offered Direct3D 12, and OpenGL for hardware without a Direct3D 12 driver.
//!
//! Two shader compilers lost the device before the window opened ("Parent device is lost", #965,
//! #1438), both on wgpu's indirect-call validation shaders, which wgpu compiles with every device:
//! Direct3D 12 took the first `dxcompiler.dll` on the search path, another program's, and failed
//! on the missing `dxil.dll` beside it; Metal's Intel compiler aborts on them on older Macs. So
//! Direct3D 12 compiles with FXC, which is part of Windows, and the validation shaders are left
//! out: FigCraft draws nothing indirectly, so they would check nothing.
//!
//! When the graphics still fail before the window's first frame, the app starts again with the
//! next of its [`attempts`]: on Windows Vulkan, then OpenGL; elsewhere OpenGL; and last the first
//! backends again with lower requirements on the power-saving GPU (macOS has no OpenGL in wgpu, so
//! that is its only other try). The attempts are a fixed list, so this ends; the message the last
//! one shows names what was tried.
//!
//! `WGPU_BACKEND` (`dx12`, `vulkan`, `gl`, `metal`) still names the backend on every system, and
//! is then the one attempt. `WGPU_DX12_COMPILER` and `WGPU_VALIDATION_INDIRECT_CALL` still choose
//! the compiler and the validation.

use std::sync::Arc;
use std::time::Duration;

use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup};
use eframe::wgpu::{self, Backends, Dx12Compiler, InstanceFlags, Limits, PowerPreference};

/// Which of the [`attempts`] this start is: set by the app itself when it starts again.
pub const ATTEMPT_ENV: &str = "FIGCRAFT_GRAPHICS_ATTEMPT";

/// Frames drawn before a failure below which the window was still starting up: the second frame
/// begins once the first was presented.
pub const STARTUP_FRAMES: u64 = 2;

/// Direct3D 12, then OpenGL: wgpu ranks a hardware GPU on either ahead of Direct3D's software one.
const WINDOWS: Backends = Backends::DX12.union(Backends::GL);

/// The graphics APIs by name, in the order the startup message lists them.
const NAMES: [(Backends, &str); 4] =
    [(Backends::DX12, "Direct3D 12"), (Backends::METAL, "Metal"), (Backends::VULKAN, "Vulkan"), (Backends::GL, "OpenGL")];

/// One way of starting the graphics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attempt {
    /// The backends wgpu chooses from.
    pub backends: Backends,
    /// The power-saving GPU and only the device limits OpenGL ES 3 hardware has.
    pub lower: bool,
}

/// The ways to start, in order: only `WGPU_BACKEND`'s backends (`requested`) when it is set;
/// otherwise [`WINDOWS`] on Windows, else eframe's `defaults`, then Vulkan on Windows, OpenGL
/// except on macOS, and those first backends again with lower requirements.
pub fn attempts(requested: Option<Backends>, windows: bool, macos: bool, defaults: Backends) -> Vec<Attempt> {
    let normal = |backends| Attempt { backends, lower: false };
    if let Some(requested) = requested {
        return vec![normal(requested)];
    }
    let first = if windows { WINDOWS } else { defaults };
    let mut list = vec![normal(first)];
    if windows {
        list.push(normal(Backends::VULKAN));
    }
    if !macos && first != Backends::GL {
        list.push(normal(Backends::GL));
    }
    list.push(Attempt { backends: first, lower: true });
    list
}

/// Which of `count` attempts this start is, from [`ATTEMPT_ENV`] (`value`): the first when it is
/// unset or not a number, the last when it is past the end.
pub fn current(value: Option<&str>, count: usize) -> usize {
    value.and_then(|v| v.trim().parse().ok()).unwrap_or(0).min(count.saturating_sub(1))
}

/// The attempt to start again with after `attempt` of `count` ended: the next one, when the
/// graphics failed (`graphics_failed`) before the window's first frame and one is left.
pub fn next(attempt: usize, count: usize, graphics_failed: bool, frames: u64) -> Option<usize> {
    let next = attempt + 1;
    (graphics_failed && frames < STARTUP_FRAMES && next < count).then_some(next)
}

/// How long this start waits for its control port: one started again ([`ATTEMPT_ENV`]) starts
/// beside the app that is ending, which still holds it for a moment.
pub fn port_wait() -> Duration {
    if std::env::var_os(ATTEMPT_ENV).is_some() { Duration::from_secs(5) } else { Duration::ZERO }
}

/// The graphics APIs `attempts` tried among those this build has (`built`), by name.
pub fn names(attempts: &[Attempt], built: Backends) -> String {
    let tried = attempts.iter().fold(Backends::empty(), |all, a| all | a.backends) & built;
    NAMES.iter().filter(|(b, _)| tried.contains(*b)).map(|(_, name)| *name).collect::<Vec<_>>().join(", ")
}

/// Set up `options` for `attempt` (see the module docs).
pub fn apply(options: &mut WgpuConfiguration, attempt: Attempt) {
    let WgpuSetup::CreateNew(create) = &mut options.wgpu_setup else { return };
    let instance = &mut create.instance_descriptor;
    instance.backends = attempt.backends;
    instance.backend_options.dx12.shader_compiler = Dx12Compiler::from_env().unwrap_or(Dx12Compiler::Fxc);
    instance.flags = instance.flags.difference(InstanceFlags::VALIDATION_INDIRECT_CALL).with_env();
    if attempt.lower {
        create.power_preference = PowerPreference::from_env().unwrap_or(PowerPreference::LowPower);
        create.device_descriptor = Arc::new(|adapter| wgpu::DeviceDescriptor {
            label: Some("egui wgpu device"),
            // The window's surface still needs textures as large as the adapter allows.
            required_limits: Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        });
    }
}

/// The attempts of this run and which one it is.
pub struct Plan {
    attempts: Vec<Attempt>,
    current: usize,
}

impl Plan {
    /// Set up `options` for this start's attempt.
    pub fn configure(options: &mut WgpuConfiguration) -> Self {
        let defaults = match &options.wgpu_setup {
            WgpuSetup::CreateNew(create) => create.instance_descriptor.backends,
            WgpuSetup::Existing(_) => Backends::all(),
        };
        let attempts = attempts(Backends::from_env(), cfg!(windows), cfg!(target_os = "macos"), defaults);
        let current = current(std::env::var(ATTEMPT_ENV).ok().as_deref(), attempts.len());
        if let Some(attempt) = attempts.get(current) {
            apply(options, *attempt);
        }
        Self { attempts, current }
    }

    /// The window opened on `adapter`: which GPU draws is the first question in every report.
    pub fn opened(&self, adapter: &wgpu::AdapterInfo) {
        let text =
            format!("graphics: drawing with {:?} on {} ({:?}, driver {})", adapter.backend, adapter.name.trim(), adapter.device_type, adapter.driver);
        if self.current == 0 {
            log::info!("{text}");
        } else {
            log::warn!("{text}, start {} of {}: {:?}", self.current + 1, self.attempts.len(), self.attempts.get(self.current));
        }
    }

    /// After the window's run (`result`, after `frames` frames): start again with the next attempt
    /// when the graphics failed while the window was starting up, otherwise say why it failed
    /// ([`crate::startup::failed`], `quiet` as there).
    pub fn finish(&self, result: eframe::Result, frames: u64, quiet: bool) -> eframe::Result {
        let Err(error) = &result else { return result };
        if let Some(next) = next(self.current, self.attempts.len(), matches!(error, eframe::Error::Wgpu(_)), frames) {
            log::warn!("graphics: {error}; starting again with {:?}", self.attempts.get(next));
            match restart(next) {
                Ok(()) => return Ok(()),
                Err(e) => log::error!("graphics: starting again failed: {e}"),
            }
        }
        let last = self.current > 0 && self.current + 1 == self.attempts.len();
        let tried = last.then(|| names(&self.attempts, wgpu::Instance::enabled_backend_features()));
        crate::startup::failed(error, quiet, tried.as_deref());
        result
    }
}

/// Start the app again with the same arguments as attempt `attempt`. On Unix the new app replaces
/// this process (an AppImage keeps its files mounted), and this returns only on failure; elsewhere
/// it starts beside this one, which then exits.
fn restart(attempt: usize) -> std::io::Result<()> {
    log::logger().flush();
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command.args(std::env::args_os().skip(1)).env(ATTEMPT_ENV, attempt.to_string());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        Err(command.exec())
    }
    #[cfg(not(unix))]
    command.spawn().map(|_| ())
}
