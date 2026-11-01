# Workspace 与依赖边界

本阶段将原单包实现移动到三个 crate，根包 `cmvs-rs` 保留既有命令和兼容库入口。
拆分阶段仅整理结构和共享资源接口，不增加指令或改变解码算法。下文回归数量与阶段停点保留为历史记录，最新运行范围见[游戏档案](games/amatsutsumi/README.md)。

| 包 | 位置 | 职责与依赖 |
|---|---|---|
| cmvs-core | crates/cmvs-core | 格式、VM、命令语义、场景、动作输入、宿主契约和标准文件系统适配器；无依赖 |
| cmvs-audio | crates/cmvs-audio | Vorbis 解码、原生音频和源时钟；依赖 core、lewton、rodio |
| cmvs-desktop | crates/cmvs-desktop | 窗口、原始输入、桌面运行入口；依赖 core 和窗口库，可选依赖 audio |
| cmvs-native | crates/cmvs-native | 操作系统服务统一接口，按平台分模块（现有 Windows，其余预留）；依赖 raw-window-handle，Windows 上 windows-sys；唯一含 FFI 的包 |
| cmvs-probe | crates/cmvs-probe | 与游戏无关的无头整机检查；依赖 core 与 desktop 的字体模块，不打开窗口或设备；不在默认成员中 |
| cmvs-rs | 根包 | CLI 目标、兼容重导出、现有集成测试；默认仅依赖 core |

依赖方向为 `desktop → core`、`desktop → audio → core`。core 的清单没有 dependencies 或平台 feature，编译器不允许核心调用音频/窗口实现。
格式、VM、场景和软件呈现继续作为 core 内部模块，未进一步拆包。
core 中的 std 文件读取器可在各平台编译，不含原生窗口或设备 API；其他平台构建仍待验证。

根 `cmvs_rs` 重导出 core；原 `cmvs_rs::desktop`、`audio_output`、`vorbis` 在对应 feature 下仍可用。
现有检查工具和 `cmvs-desktop` 二进制名不变，桌面 CLI 目标只调用子 crate 的运行入口。
根集成测试保留原路径，继续同时验证公开兼容入口和 CLI；合成 fixture 不移动、不新增真实素材。

## 功能选择与检查

根 feature 名保持 `desktop` 和 `audio-output`。`audio-output` 通过弱依赖 feature 转发给已启用的 desktop，单独启用音频不会拉入窗口库。
workspace `default-members = ["."]`，因此原来的根目录命令只选择根包，默认仍无第三方依赖。
`unsafe_code = forbid`、严格 Clippy 约定由 workspace 集中定义，四个原有包均继承；`cmvs-native` 改为 deny，只在注明 SAFETY 的系统调用处局部允许 unsafe。
外部依赖版本集中在 workspace.dependencies，Cargo.lock 保留原 registry 包版本与校验值。

```bash
# 原来的命令继续有效
cargo fmt --check
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
cargo run --offline --release --features desktop,audio-output --bin cmvs-desktop -- "E:/Download/AMATSUTSUMI" --encoding shift-jis --continue-without-snapshot --audio native

# 覆盖所有子包、后端及兼容入口
cargo fmt --all --check
cargo test --offline --workspace --all-features
cargo clippy --offline --workspace --all-targets --all-features -- -D warnings

# 独立检查核心与不启用音频的桌面包
cargo tree --offline -p cmvs-core
cargo check --offline -p cmvs-desktop --no-default-features
```

直接构建 workspace 全体成员会构建音频和桌面成员；这不表示它们成为 core 或根包默认依赖。
直接使用库的调用方可依赖所需子 crate；既有调用方可以继续使用根兼容入口。

## 通用资源读取器

`cmvs_core::resources` 提供 `ResourceArchives`、`ResourceSource`、`ResourceKind` 和注册视图。
三种资源族明确绑定图像 0x2011 的十槽、SE 0x2012 的四槽、BGM 0x2013 的四槽，不能用任意 slot_count 绕过这些限制。
按注册槽号查找第一个命中，目录缺失/归档缺失为 miss，坏归档、校验失败与路径逃逸为错误。
缓存键为 `(资源族命令, 槽号)`，路径重新登记时重开；不同资源族使用相同槽号时互不覆盖缓存。
目录读取上限、CPZ6 查找/校验算法和路径约束沿用原实现。

`image::ImageArchives` 只负责绑定图像资源视图，保留原公开 API；图像解码分派仍在 image 模块。
`cmvs-audio` 直接使用通用资源接口，不再依赖 image 模块或 crate-private 的辅助函数。
原版地址证据仍以 research.md 的二进制哈希为准；未新推断指令或格式。

## 回归证据

三个新增合成测试验证资源族/槽数隔离、SE 搜索顺序和重新登记、音频路径约束；已有图像 CPZ 测试继续验证校验和与重开行为。
默认 272 项、desktop 277 项、完整功能 282 项测试；全 workspace 严格 Clippy 和格式检查通过。

拆分前后从真实正常入口运行固定种子 0、16ms、160000 步的 trace，19111724 字节输出逐字节相同，
SHA-256 `de61dfb967c153cf34c768b9145a7f85e9c78cb406308280d1627e425380067d`。
两次均按设计在研究步数耗尽时返回 1，覆盖标题 → OP → 返回标题的流程。
搬迁的既有核心模块（image 与 lib 的结构调整除外）经文件哈希确认内容未变；CPZ 加密子模块也保持原样。

拆分后 `bgm41v.ogg` 的完整 f32 PCM SHA-256 仍为 `3b25dc9fe8407249a4179ba36bbb5c8aab77bb0f64e16d7905dcba35ead8b99f`；
OP 第 1000 帧 RGBA 仍为 `ece6dcbe6d8365ef6356a69b83e8a76c9b437a191c248d15c9dba357cb289da0`，与拆分前 Python 对照结果一致。
真实输入始终只读，trace、PCM 和图像输出保留在忽略目录中，不进入 Git。

拆分后的 Windows 原生桌面以音频模式运行 24 秒，成功准备/启动 SE、读取 BGM 前奏与循环段、显示完整标题，22303 步后退出码 0。
与上一阶段结束时的标题帧进行文件对照；原始输入仍只观察、不驱动 VM。

字体预览阶段：cmvs-desktop 直接依赖固定 ab_glyph 0.2.32（仅 std，默认功能关闭），字体文件/字符编码/光栅化/字形缓存都留在桌面 crate。core 只新增可接收外部 SceneObject 的排序呈现入口，仍无依赖；没有依赖图反向引用。Cargo.lock 只新增 desktop → ab_glyph 的直接依赖边，复用已锁定的版本。默认 356 项、全 workspace 全功能 373 项测试通过，详见 font-output.md。


## 场景内部整理（2026-10-28）

保持四个 workspace 包与依赖方向，未新增 feature 或第三方依赖。`scene.rs` 的帧动画类型/计算迁入 `scene/animation.rs`，不透明度轨道与命令迁入 `scene/opacity.rs`，原 `scene::Animation`、`AnimationFrame`、`ANIMATION_FRAMES`、`OpacityKey`、`OpacityTrack`、`OPACITY_KEYS` 路径均由重导出保留。算法、字段及效果推进顺序不变。

`SequenceHost` 只决定动作执行时机与快进跳过，正常执行通过 crate 内部 `SceneHost::replace_world_motion` 重建并启动轨道。方法留在 `scene/world_motion.rs`，与直接播放命令共用私有 `WorldMotion::start`，不把原始 DrawState 字段和轨道构造留给序列层。

`tests/scene.rs` 与 `tests/sequence.rs` 共用 `tests/support/bytecode.rs` 的合成 PS2A、参数顺序命令构造器和等待辅助函数。`command` 的 args[0] 对应 SP-4，按逆序压栈；场景测试原有 `invoke` 仍接收实际压栈顺序，明确标注差异。未迁移其他测试或加入游戏数据。

重构前后鼠标、Enter、Ctrl 重放日志逐字节一致：两条点击路径分别 8479 字节、SHA-256 `cf836f8439223d2c63fb6000eeae451140c9d70f2042f21887819f1c6659e8f9`，Ctrl 1072 字节、SHA-256 `b02191202dbffd82a76ac1564b3243b0ff599efd22eadf071f25d1fe82a25ae8`；15 张导出 PNG 逐字节一致。此处使用与上一阶段相同的抽样呈现探针，未扩展为全部让出帧验证。当前停点仍为 `sn1002.ps3 / 0x267b4 / 0x21fa` 的模式 0x48。

默认 526 项、全 workspace 全功能 557 项测试、格式、默认/全功能严格 Clippy 与完整功能桌面 release 构建通过，硬件检查 1 项默认忽略。未修改资源解码算法、指令语义或游戏数据，未新增原生窗口实测。

## 用户目录后端与工具链（2026-10-29）

workspace 各包声明 Rust 1.89+（本机使用 1.96.0），用于标准库 File::try_lock 的跨进程会话锁，无新增依赖或 unsafe。system_save/profile.rs 为标准库文件系统适配器，处理独立用户目录的主文件、备份、临时文件提交及锁；原有编码/解码和游戏目录只读边界不变。default/全功能 Windows 检查数量为 587/632，另有 1 项音频设备测试默认忽略；其中 1 项主文件分享模式导致替换失败的回归检查仅在 Windows 编译。平台无关存储测试也在 Windows 上执行，其他平台尚未构建验证。

## 引擎档案（2026-10-30）

`cmvs-core` 新增 `digest`（SHA-256）、`exe`（只读 PE 与 switch 定位）和 `profile` 模块，仍无第三方依赖。档案表与游戏记录是生成的元数据，放在 `crates/cmvs-core/profiles/`，以 `include_str!` 内嵌，不含原版代码或素材。根包新增 `cmvs-profile` 二进制（`identify`/`table`/`show`）。默认 664 项、全 workspace 全功能 712 项测试通过，另有 1/2 项默认忽略。

## 可选 GPU 后端

`cmvs-wgpu` 依赖 core、固定 wgpu/pollster，封装设备、纹理缓存、场景合成、窗口呈现和逻辑帧读回；desktop 在 `wgpu-renderer` 下依赖它。根 feature 同名，自动启用 desktop；默认成员与 core 依赖不变。`cargo check --offline --features wgpu-renderer` 检查 GPU 构建，`cargo check --offline -p cmvs-desktop --no-default-features` 检查 CPU-only。全 workspace 全功能测试仍需另显式执行忽略的GPU硬件检查。

锁文件把 gpu-allocator 的宽范围 windows 依赖绑定到0.62.2，与 wgpu-hal 一致；cpal 0.18.2 也解析到同一 windows 0.62.2，不再保留旧的 0.54 分支。更新锁文件后仍应检查 GPU/音频全功能构建，避免宽范围解析把两套类型错误合并。

## Linux 字体服务（2026-10-31）

`cmvs-native` 在 Linux 上通过 Fontconfig 的 `fc-list` 发现字体目录、通过 Zenity 选择日文字体族；字体文件解析仍留在 `cmvs-desktop`，平台无关核心不变。Wayland 标题启用 winit 的 crossfont 后端，逐字使用系统字体回退。Linux 构建需要 FreeType/Fontconfig 开发包，原生音频另需 ALSA 开发包（Ubuntu: `libfreetype-dev libfontconfig-dev libasound2-dev`）；字体选择运行时需 `fontconfig zenity` 及日文字体（例如 `fonts-noto-cjk`）。

## 依赖升级（2026-11-02）

在已安装系统开发包的 Linux 构建上升级四个固定版本：pollster 0.4.0→1.0.1、encoding_rs 0.8.42→0.8.44、ico 0.5.0→0.6.0、rodio 0.21.1→0.23.0；cpal 随 rodio 由 0.16.0 升至 0.18.2，lewton、winit、softbuffer、ab_glyph、wgpu 保持原版本。rodio 0.22 起把 OutputStream/OutputStreamBuilder/Sink 改名为 MixerDeviceSink/DeviceSinkBuilder/Player，`Source::channels/sample_rate` 改为返回 NonZero；`cmvs-audio` 只做对应改名与类型包装，播放、混音、源时钟算法未变。ico 0.6.0 的读取/解码调用方式不变，其新增文件范围校验位于既有预检之后。

升级后格式检查、默认 959 项与全 workspace 全功能 1077 项测试（25 项忽略）、默认与全功能严格 Clippy 及 `desktop,audio-output` release 构建通过；Linux 原生音频设备以 48 kHz/2 声道打开，aoitori 运行到诊断退出，未做听感或音画同步实测。版本、提交与许可已同步到 references.md 和 THIRD_PARTY_NOTICES.txt；历史游戏记录中的旧版本描述保留为当时事实。
