# cmvs-wgpu

CMVS 的可选 GPU 合成后端，依赖 `cmvs-core`、`wgpu` 和 `pollster`。消费 Core 生成的绘制队列，管理 GPU 资源、场景合成、窗口呈现与逻辑帧读回。

主要职责：

- 通过 `Renderer` 创建设备与渲染管线，缓存纹理并执行已支持的绘制与混合操作。
- 将逻辑画布缩放到窗口表面，保持引擎坐标与宿主窗口尺寸分离。
- 用 `GpuFrame` 保存已提交逻辑帧，在截图或设备画面查询时按需读回像素。

VM、场景状态、绘制顺序和命令语义由 Core 决定，本包不扩展游戏指令支持范围。当前已做 Windows Vulkan/DX12 部分硬件与画面对照，尚未覆盖完整路线或跨平台 GPU 行为；桌面默认仍使用 CPU。

从仓库根目录检查或启用：

```bash
cargo check --offline -p cmvs-wgpu
cargo check --offline --features wgpu-renderer
```

根包的 `wgpu-renderer` feature 自动启用桌面及本后端，运行时还需指定 `--renderer wgpu`。GPU 硬件测试默认忽略，需在具备适配器的环境中显式执行；常规编译检查不代表设备验证通过。

详见[wgpu 后端](../../docs/backends/wgpu.md)、[桌面后端](../../docs/backends/desktop.md)和[依赖与许可](../../docs/references.md)。
