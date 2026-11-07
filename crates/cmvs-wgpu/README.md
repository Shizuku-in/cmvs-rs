# cmvs-wgpu

GPU backend that executes the `cmvs-core` draw queue with `wgpu` instead of the software compositor.

## API

- `Renderer` checks the device, renders a `RenderFrame` into a `GpuFrame` and caches textures by content.
- `GpuFrame::read` reads the logical frame back to RGBA. It is only called when the engine asks for a capture.
- `Presenter` scales a `GpuFrame` to a window surface, separately from the logical canvas.

## Features

| Feature | Adds |
|---|---|
| `webgpu` | Browser WebGPU backend, used by the Web player from a worker's `OffscreenCanvas` |

Native targets use the DX12, Metal and Vulkan backends and block on device creation with `pollster`.

## Used by

`cmvs-desktop` (`wgpu-renderer` feature), and the Android and Web players.
