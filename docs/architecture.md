# 架构与开发

## Workspace

| 包 | 职责 |
|---|---|
| `cmvs-core` | 格式、资源、VM、命令语义、场景、软件呈现与宿主契约 |
| `cmvs-input` / `cmvs-font` | 物理输入转换；文字编码、字体解析与光栅化 |
| `cmvs-audio` / `cmvs-video` | 音频解码、混音与输出；Theora 解码与预解码 |
| `cmvs-wgpu` | GPU 场景合成、纹理缓存与读回 |
| `cmvs-native` | 各平台的系统服务与原生 FFI |
| `cmvs-host` | 跨平台会话层、不拥有窗口的播放器与 C ABI |
| `cmvs-desktop` | 桌面窗口、输入、呈现与运行入口 |
| `cmvs-console` | Switch / Vita 共用的手柄、页面与 GPU 合成策略 |
| `cmvs-cli` | 格式检查、VM trace 与桌面 CLI 入口 |
| `cmvs-disassembler` | 脚本反汇编、可编辑项目和翻译表导出 |
| `cmvs-assembler` | 脚本汇编、资源转换、封包编辑与整作重建 |
| `cmvs-tooling` | 汇编器与反汇编器共用的脚本布局、翻译格式和文件操作 |
| `cmvs-profile-research` | 源档案校验、PE 分派分析与运行表生成 |
| `cmvs-patch` | 补丁资产提取、重编码与转换 |
| `cmvs-probe` / `cmvs-bench` | 无头运行、路线、输入与存读档检查；热路径微基准测试 |
| `platforms/*` | 各平台宿主和构建入口，见[平台总览](../platforms/README.md) |

核心定义语义和后端契约，仅依赖 Rust 标准库。宿主提供窗口、设备、字体和输出；Android 和 Web 的文件系统适配位于宿主层。

共享依赖在根 `Cargo.toml` 声明，精确版本由 `Cargo.lock` 固定。

## 执行流程

1. `profile::Registry` 根据游戏数据和二进制身份选择档案；`bootstrap` 应用配置并登记资源。
2. `machine::Machine` 执行 VM；系统命令先经 `CommandGate` 选择语义，再由共享 `dispatch` 分派。
3. `runtime::Runtime` 编排脚本、资源、输入、场景、音频和存档。宿主在 VM 让出后推进帧时间，合成并呈现画面。
4. 完整嵌入播放器使用 `cmvs-host::Player`；自定义宿主使用 `Runtime`。调用顺序和示例见 [API 指南](guides/api.md)及源码 rustdoc。

VM 预算耗尽不代表帧完成：续跑时保留当前输入，不重复推进时间。字体、影片和场景合成由宿主完成；请求的设备帧与延迟截图按[后端契约](backends/platform-host.md)提供。

## 维护边界

- 格式解析保留边界检查、原始文本字节和明确的像素格式。未知状态或未支持模式返回错误。
- 构建差异由[引擎档案](design/engine-profiles.md)选择，宿主不按游戏身份分支。命令开发见[分派指南](guides/dispatch.md)。
- core 保存引擎状态，不持有窗口或图形设备；CPU / GPU 消费共享绘制队列，见 [GPU 后端](backends/wgpu.md)。
- workspace 默认禁止 unsafe；平台 FFI 和视频私有 SIMD 内核局部放行，注明 SAFETY，并保留相应验证。
- 游戏支持范围只记在[游戏档案](games/README.md)，平台构建与操作只记在对应平台 README。

## 检查

在仓库根目录运行：

```sh
cargo fmt --all --check
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
```

涉及子包、后端或 feature 时追加：

```sh
cargo test --offline --workspace --all-features
cargo clippy --offline --workspace --all-targets --all-features -- -D warnings
cargo check --offline -p cmvs-desktop --no-default-features
```

默认命令检查 workspace 的默认成员及功能；`-p <包名>` 可仅检查相关包。跨平台变更还需对应目标构建和硬件验证。系统依赖见[平台总览](../platforms/README.md)，外部样本检查见[研究约定](research.md)。
