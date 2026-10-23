# FigCraft patch to wgpu-hal 30.0.1

Source: https://crates.io/crates/wgpu-hal/30.0.1 (MIT OR Apache-2.0; licenses are alongside this file).

FigCraft changes `src/dx12/mod.rs`: the `DXGI_SWAP_CHAIN_DESC1` uses
`DXGI_SCALING_NONE` for HWND swap chains and retains `DXGI_SCALING_STRETCH`
for composition swap chains. No other upstream source is changed. The Cargo
`[patch.crates-io]` entry in the workspace manifest applies this to the
existing wgpu dependency graph.

On the affected Windows system, `DXGI_SCALING_STRETCH` resamples FigCraft's
windowed UI even though the renderer's framebuffer is sharp. With `NONE`,
the presented window and framebuffer match pixel for pixel at 1440×900 and
1000×750, including after restoring from a maximized window. See FigCraft
issue #778 for the reproduction and measurements.
Microsoft requires `DXGI_SCALING_STRETCH` for
`CreateSwapChainForComposition`, so the patch preserves it for composition
targets: https://learn.microsoft.com/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgifactory2-createswapchainforcomposition

Upstream tracking: https://github.com/gfx-rs/wgpu/issues/10196 discusses
configurable swap-chain scaling, including the DX12 backend's hard-coded
`DXGI_SCALING_STRETCH`. Its Vulkan symptom is different from FigCraft's DX12
blur, so the issue is a tracking point for the scaling policy, not proof that
the upstream fix has landed.

Remove this vendor copy once a released wgpu-hal supports non-stretch DX12
presentation (or exposes a scaling mode that FigCraft can select), and FigCraft
has upgraded to and verified that release.
