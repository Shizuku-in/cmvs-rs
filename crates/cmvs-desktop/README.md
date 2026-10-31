# cmvs-desktop

CMVS 的桌面宿主库，负责窗口、物理输入、字体输出和应用事件循环。依赖 `cmvs-core` 与 `cmvs-native`，按功能开关接入音频和 GPU 后端；同名可执行程序由仓库根包提供。

主要职责：

- 将键鼠与窗口事件转换为 Core 的输入和宿主请求，驱动 VM 与完整帧更新。
- 解析字体与文本编码、生成字形，并呈现场景和文字。
- 协调独立用户目录、运行设置与退出流程，通过 `cmvs-native` 调用系统服务。

功能开关默认关闭：

| Feature | 用途 |
|---|---|
| `audio-output` | 接入 `cmvs-audio`，运行时以 `--audio native` 打开设备 |
| `wgpu-renderer` | 接入 `cmvs-wgpu`，运行时以 `--renderer wgpu` 选择 GPU；默认使用 CPU |

从仓库根目录运行：

```bash
cargo check --offline -p cmvs-desktop --no-default-features
cargo run --offline --release --features desktop --bin cmvs-desktop -- "<游戏目录>" --encoding shift-jis --record-text
```

示例使用文字记录模式；显示字形需按运行指南配置字体。当前属于研究预览，Windows 已有部分实测，完整游戏流程及其他平台仍待验证。原游戏保持只读，存档写入需显式配置独立用户目录。

详见[运行指南](../../docs/guides/running.md)、[桌面后端](../../docs/backends/desktop.md)和[字体输出](../../docs/backends/font-output.md)。
