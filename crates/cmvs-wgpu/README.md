# cmvs-wgpu

GPU backend that executes the `cmvs-core` draw queue with `wgpu` instead of the software compositor.

## API

- `Renderer` checks the device, renders a `RenderFrame` into a `GpuFrame` and caches textures by content.
- `GpuFrame::read` reads the logical frame back to RGBA. It is only called when the engine asks for a capture.
- `Presenter` scales a `GpuFrame` to a window surface, separately from the logical canvas.

Canvas textures are pooled by size (up to 4 entries / 64 MiB), and movie upload textures have a separate pool (up to 8 entries / 128 MiB). A retained `GpuFrame` owns a lease, so snapshots and previously presented frames cannot be overwritten. Raw frame texture handles stay inside this backend. Released textures can be reused through the same queue without a device idle wait; requests beyond the pool limits remain transient while all slots are leased. These limits describe pool retention, not total GPU memory.

Window and movie scaling keep their uniform buffers and up to 16 texture bindings. Unchanged parameters skip uploads; bindings use weak owners so they do not pin evicted textures. Each scene pins its movie inputs until submission, including when the movie cache evicts entries during draw preparation.

## Features

| Feature | Adds |
|---|---|
| `webgpu` | Browser WebGPU backend, used by the Web player from a worker's `OffscreenCanvas` |

Native targets use the DX12, Metal and Vulkan backends and block on device creation with `pollster`.

## Used by

`cmvs-desktop` (`wgpu-renderer` feature), and the Android and Web players.
