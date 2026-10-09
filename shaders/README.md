# Shared WGSL shaders

Compiled by `debut-render` through wgpu for Metal, Vulkan, DX12 and WebGPU (NFR-09, PLT-04).
Desktop and browser renders of the same graph must match within the tolerance defined in
`debut-render::graph` tests.

Planned files: `composite.wgsl`, `transform.wgsl`, `color_transform.wgsl`, `lut3d.wgsl`,
`key.wgsl`, `mask.wgsl`, `scopes.wgsl`.
