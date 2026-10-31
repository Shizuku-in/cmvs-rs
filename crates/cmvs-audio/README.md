# cmvs-audio

CMVS 的可选音频后端，负责 Vorbis 流式解码与原生设备播放。依赖 `cmvs-core`、`lewton` 和 `rodio`，不依赖桌面窗口包。

主要职责：

- 解码 Ogg/Vorbis 数据，支持已实现的 BGM 前奏与循环段播放。
- 实现 Core 的 `AudioBackend` 契约，播放 SE、BGM、语音和影片音轨。
- 执行音量、淡入淡出、语音压低 BGM 等播放请求，并提供影片音源消费时钟。

音频命令语义和引擎状态保留在 Core，本包负责解码与设备输出。当前已有 Windows 部分路径的设备检查，设备缓冲延迟尚未校准，其他平台尚未完成构建与行为验证。

从仓库根目录检查或启用：

```bash
cargo test --offline -p cmvs-audio
cargo check --offline --features desktop,audio-output
```

根包的 `audio-output` feature 引入本包；桌面播放还需在运行时指定 `--audio native`。默认记录模式不会打开音频设备，硬件测试需单独显式运行。

详见[音频后端](../../docs/backends/audio-output.md)、[运行指南](../../docs/guides/running.md)和[依赖与许可](../../docs/references.md)。
