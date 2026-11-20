# Apple（macOS / iOS）SwiftUI 启动器与播放器设计计划

> **文档状态**：设计与验收计划。SwiftUI 前端与 Rust 桥接层已接入仓库，Apple CI 与实机验收待验证。构建方法和验证范围见 [`platforms/apple/README.md`](../../platforms/apple/README.md)。

本文档规划一套面向 **macOS** 与 **iOS / iPadOS** 的统一 **SwiftUI** 启动器与播放器前端，用于替代当前的命令行测试入口与最小 UIKit 原型，并在 Apple 平台完整接入 `cmvs-wgpu` Metal 渲染、游戏库、字体管理、存档管理及视觉小说触控/键鼠交互。

---

## 1. 已确认的设计约束与选型

| 决策项 | 选定方案 | 说明 |
|---|---|---|
| **最低系统版本** | **macOS 14.0 (Sonoma)+**、**iOS / iPadOS 17.0+** | 采用 `@Observable`、`NavigationSplitView`、`WindowGroup`、`NSView.displayLink` / `CADisplayLink`、Swift Concurrency 等现代 API，不为旧系统维护兼容分支 |
| **分发方式** | **GitHub Release 直接分发** | macOS：非沙盒（Non-Sandboxed）通用 `.app`（`.dmg` / `.zip`），可原地读取任意目录并支持可选 `<game>/savedata/`；iOS：未签名 `.ipa`（供自签名/侧载），绝不打包游戏数据或受限商业字体 |
| **工程管理** | **XcodeGen**（`platforms/apple/project.yml`） | 由构建脚本动态生成 `.xcodeproj` 并加入 `.gitignore`，避免手工维护 `.pbxproj` 产生合并冲突 |
| **开发与验证路径** | **Linux 开发 + GitHub Actions 编译验证** | 先在 Linux 完成 Rust 桥接层可移植逻辑与单元测试，再通过 GitHub Actions `macos-latest` 执行交叉编译、XCFramework 打包、`xcodegen`、`xcodebuild` 与 Swift 测试 |

---

## 2. 现状盘点与目标

### 2.1 改造前的入口
- **macOS**：仅通过 `cmvs-desktop`（[`crates/cmvs-desktop/src/app.rs`](../../crates/cmvs-desktop/src/app.rs)）提供命令行窗口入口，无图形化启动器、游戏库及 `.app` 封装。
- **iOS**：仅保留 192 行的最小 UIKit 原型 旧入口 `platforms/apple/ios/main.mm`，存在以下局限：
  - 基于 [`crates/cmvs-host/cmvs_host.h`](../../crates/cmvs-host/cmvs_host.h) 的 CPU 合成，每帧整幅拷贝 RGBA8 到 `CGImage` / `UIImageView`，未使用 `cmvs-wgpu` Metal 后端；
  - 旧构建脚本强制将单个 `--font` 打包入 `.app`，不支持 TTC 字面索引选择与多字体切换；
  - 仅映射单指点击，缺少视觉小说常用的右键、快进、滚轮履历等交互；
  - 缺少游戏库、单游戏配置与存档管理界面。

### 2.2 改造目标
- 打造 **SwiftUI 多平台启动器**，统一管理游戏库、字体、存档与诊断配置；
- 在 macOS 与 iOS 上均默认启用 **`cmvs-wgpu` Metal GPU 合成**（保留可选 CPU 路径），并严格遵守引擎帧边界；
- 提供贴合桌面键鼠与移动端触控的完整视觉小说交互闭环。

---

## 3. 总体架构与目录规划

### 3.1 分层架构

```text
┌──────────────────────────────────────────────────────────────────────┐
│               SwiftUI Multiplatform App (macOS + iOS)                │
│  ┌─────────────────────────┐      ┌───────────────────────────────┐  │
│  │ 启动器界面               │      │ 播放器视图                     │  │
│  │ 游戏库 / 字体 / 存档管理  │      │ CAMetalLayer + 键鼠/触控手势   │  │
│  │ 单游戏配置 / 诊断信息     │      │ 快捷浮层 / 原生弹窗 / 错误页    │  │
│  └────────────┬────────────┘      └───────────────┬───────────────┘  │
│               └─────────────────┬─────────────────┘                  │
│                                 ▼                                    │
│                 Swift Engine Wrapper (`CmvsBridge`)                  │
└─────────────────────────────────┬────────────────────────────────────┘
                                  │ 稳定 C ABI (`cmvs_apple.h` + XCFramework)
┌─────────────────────────────────▼────────────────────────────────────┐
│          `platforms/apple/native` (workspace crate: `cmvs-apple`)    │
│  • 会话与存储：cmvs-host (`HostSession` / `GameBootstrap` / `game_savedata`) │
│  • GPU / CPU：cmvs-wgpu (Metal backend -> CAMetalLayer) + CPU 合成   │
│  • 输入映射：cmvs-input (`TouchInput`) + cmvs-host (`InputState`)    │
│  • 媒体与核心：cmvs-font / cmvs-audio / cmvs-video / cmvs-core       │
└──────────────────────────────────────────────────────────────────────┘
```

### 3.2 为什么新建 `cmvs-apple` 而不是扩展 `cmvs_host.h`
1. **保持既有 ABI 稳定**：[`crates/cmvs-host/cmvs_host.h`](../../crates/cmvs-host/cmvs_host.h)（API v1）与 [`platforms/host/CMakeLists.txt`](../../platforms/host/CMakeLists.txt) 面向通用轻量级 C 宿主，不应绑定 Apple 专有概念（`CAMetalLayer`、游戏探测 DTO、平台事件队列）。
2. **与现有平台后端对称**：参考 [`platforms/android/native/Cargo.toml`](../../platforms/android/native/Cargo.toml) 的分层方式，`cmvs-apple` 组合 [`cmvs-host`](../../crates/cmvs-host/README.md)（`std-fs` feature）、[`cmvs-wgpu`](../../crates/cmvs-wgpu/Cargo.toml)、[`cmvs-input`](../../crates/cmvs-input/src/lib.rs)、`cmvs-font`、`cmvs-audio` 与 `cmvs-core`，不引入 `uniffi` 等额外代码生成工具链，保持 [`Cargo.lock`](../../Cargo.lock) 精简。
3. **遵守仓库安全边界**：符合 [`docs/architecture.md`](../architecture.md) 约定，仅在 `ffi.rs` 与 Metal 表面创建处局部使用 `#[allow(unsafe_code)]` 并注明 `SAFETY`，其余代码保持安全 Rust。

### 3.3 规划目录结构

```text
platforms/apple/
├── README.md                        # Apple 构建、签名、操作与验证文档
├── build.sh                         # Rust 交叉编译 -> CmvsApple.xcframework -> xcodegen -> xcodebuild
├── project.yml                      # XcodeGen 工程声明（macOS + iOS + Tests）
├── native/                          # 新增 workspace 成员：cmvs-apple (rlib + staticlib)
│   ├── Cargo.toml
│   ├── include/
│   │   ├── cmvs_apple.h             # C ABI 头文件
│   │   └── module.modulemap         # 供 Swift 直接 import CmvsAppleC
│   └── src/
│       ├── lib.rs                   # AppleSession（协调 VM、渲染、音频与输入）
│       ├── backend.rs               # 实现 PlatformBackend（标题、显示偏好、光标与弹窗事件）
│       ├── inspect.rs               # 游戏探测、可用档案导出、字体预检
│       ├── renderer.rs              # Metal (cmvs-wgpu on CAMetalLayer) + CPU 帧缓冲路径
│       ├── input.rs                 # macOS HID keyCode/鼠标 与 iOS TouchInput 映射
│       └── ffi.rs                   # C ABI 导出（catch_unwind 与错误字符串缓冲）
└── app/                             # SwiftUI 多平台应用源码
    ├── Shared/
    │   ├── App/                     # @main App 入口、全局状态与导航容器
    │   ├── Bridge/                  # Swift 安全封装（CmvsEngine, CmvsInspector, CmvsError）
    │   ├── Models/                  # GameItem, GameSettings, FontRecord, SaveSummary
    │   ├── Store/                   # 游戏库持久化、Security-Scoped Bookmark、字体与存档服务
    │   ├── Views/
    │   │   ├── Library/             # 游戏网格/列表、详情与单游戏配置
    │   │   ├── Fonts/               # 字体列表、CoreText TTC 字面解析、实时预览
    │   │   ├── Saves/               # 存档概览、备份导入/导出
    │   │   ├── Settings/            # 全局设置与高级诊断选项
    │   │   └── Player/              # 播放器容器、评论输入、游戏内字体选择器、结构化错误页
    │   └── Resources/               # 本地化文案（简体中文 / English / 日本語）、AppIcon、Info.plist
    ├── macOS/
    │   ├── MetalPlayerView.swift    # NSViewRepresentable + CAMetalLayer + 键鼠/滚轮/NSCursor
    │   ├── MacCommands.swift        # 菜单栏命令（⌘O、⌘F、⌘S 截图、⌘W 保存并退出）
    │   └── MacPlatformServices.swift # NSOpenPanel、Finder 显示、系统字体目录扫描
    ├── iOS/
    │   ├── MetalPlayerView.swift    # UIViewRepresentable + CAMetalLayer + UITouch/iPadOS 外设
    │   ├── TouchOverlayView.swift   # 可折叠视觉小说快捷浮层
    │   └── IOSPlatformServices.swift # UIDocumentPicker、目录复制导入器、AVAudioSession 管理
    └── Tests/                       # Swift 单元测试（书签、导入校验、配置迁移与编码校验）
```

---

## 4. 核心技术设计

### 4.1 Rust C ABI（`cmvs_apple.h`）

除播放控制外，`cmvs-apple` 为启动器补充启动前探测与事件拉取能力：

- **启动前探测（异步在后台线程调用）**
  - `cmvs_apple_inspect_game(path, profile_opt, encoding_opt)`：复用 [`GameBootstrap::load`](../../crates/cmvs-host/src/session.rs#L204-L227) 校验 `data/pack/script.cpz` 与 `start.ps3` / `start.ps2` / `start.pss`，返回识别到的游戏 ID、引擎家族（`CMVS` / `CVNS`）、匹配依据、自动判定的文本编码、逻辑画面尺寸及是否已存在 `<game>/savedata/`。
  - `cmvs_apple_list_profiles()`：导出 [`Registry::builtin`](../../crates/cmvs-core/src/profile.rs#L97-L100) 的档案 ID 与游戏别名列表，供高级设置手动覆盖。
  - `cmvs_apple_validate_font(path, face_index)`：按 `cmvs-font` 的规则检查文件大小（`<= 32 MiB`）及字面可读性。
- **会话生命周期与运行配置**
  - `cmvs_apple_open(config)`：接收游戏路径、存档模式（外部用户目录 vs [`game_savedata`](../../crates/cmvs-host/src/storage.rs#L134)）、外部用户目录路径、字体路径与 TTC `face_index`、编码策略（`auto` / `shift-jis` / `gbk` / `utf-8`）、渲染器（`metal` / `cpu`）、`profile` 覆盖、`skip_auto_resume`、`continue_without_snapshot`、`block_presumed`、`block_unreviewed` 与 `native_audio` 开关。
  - `cmvs_apple_attach_metal_layer(handle, layer_ptr, width_px, height_px)` / `resize` / `detach`：管理 `CAMetalLayer` 绑定关系。
  - `cmvs_apple_tick(handle, delta_ms)`：推进一次协作预算切片，返回状态码（`BudgetExhausted`、`FrameReady`、`Quit`、`Inactive` 或错误 `-1`）。
  - **结构化事件队列**：每帧由 Swift 拉取 `TitleChanged`、`DisplayPreferencesChanged`（全屏 / 比例模式 / 线性滤波）、`CursorChanged`（RGBA + hotspot + 可见性）、`CommentInputRequested`、`FontDialogRequested`、`MessageDialogRequested` 与结构化停止诊断（脚本名、PC、步数）。

### 4.2 渲染与帧边界

- **Metal GPU 路径（默认）**：基于 [`crates/cmvs-wgpu/src/lib.rs`](../../crates/cmvs-wgpu/src/lib.rs)，在 Apple 平台启用 `wgpu::Backends::METAL`，通过 `CAMetalLayer` 创建 `wgpu::Surface` 与 `Presenter`。普通帧全程留在 GPU 合成与缩放，零 CPU 像素回读；即时截图按需调用 `GpuFrame::read()`（遵循 [`docs/backends/wgpu.md`](../backends/wgpu.md)）。
- **CPU 路径（可选）**：调用 `HostSession::compose_cpu_frame` 产出 RGBA8 缓冲，由宿主上传至 `MTLTexture` 或 `CGImage` 呈现。
- **错误显式暴露**：严格遵守仓库约定，Metal 适配器、表面创建或管线失败时直接向用户报错，**不静默回退到 CPU**，由用户在启动器设置中显式切换。
- **严格帧边界与安全析构**：
  - 每次 `tick` 使用 `FrameBudget::time(Duration::from_millis(8))`；仅在 `FrameReady` 时更新呈现与输入状态，`BudgetExhausted` 不提交半成品画面。
  - 窗口关闭或视图销毁前，必须严格按顺序执行：**停止 `displayLink` → 等待当前 `tick` 结束 → `save_preferences` → `detach_metal_layer`（销毁 `Presenter` 与 `wgpu::Surface`）→ `destroy`**，防止 `CAMetalLayer` 提前释放引发 use-after-free。

### 4.3 线程模型与生命周期

- **耗时操作离主线程**：添加游戏、扫描目录与 `cmvs_apple_open`（含大文件哈希识别与启动脚本解码）均运行在后台线程/Swift `Task.detached`，界面展示加载指示与可取消状态。
- **单句柄严格串行**：运行期所有对同一 `CmvsAppleHandle` 的调用经由专用高优先级串行通道调度，不在多线程间并发可变访问。
- **前后台与失焦状态同步**：App 进入后台、最小化或弹出系统文件选择器时，立即调用暂停、静音并清除输入按住状态（参照 [`TouchInput::focus`](../../crates/cmvs-input/src/lib.rs#L69-L74) 与 [`InputState::clear`](../../crates/cmvs-host/src/input.rs#L93-L97)，不生成误点击抬起边沿）；恢复前台时重置上一帧时间戳。

### 4.4 音频与 iOS 中断处理

- **默认路径**：使用 `cmvs-audio` 的 `native` 模式（通过 `rodio` / `cpal` 驱动 CoreAudio），并继续复用 `HostAudioControl::set_active` 在后台静音。
- **iOS `AVAudioSession` 管理**：
  - 打开游戏前设置 `AVAudioSessionCategory.playback` 并激活会话；
  - 监听 `AVAudioSession.interruptionNotification`（来电/闹钟/Siri）与 `routeChangeNotification`（耳机/蓝牙音频拔出），中断开始时暂停引擎与音频输出，中断结束后重新激活音频会话。
- **降级预案**：若实机测试发现 `cpal` CoreAudio 在长期后台挂起或多次音频路由切换后出现设备流失效无法恢复，可切换为 `cmvs-audio` 的 `mixer` 模式（参见 [`crates/cmvs-audio/README.md`](../../crates/cmvs-audio/README.md) 中的 `PcmOutput` 拉流接口），由 Swift 侧 `AVAudioEngine` + `AVAudioSourceNode` 消费混合后的 PCM。

### 4.5 字体管理与 32 MiB 限制

- [`crates/cmvs-font/src/font.rs`](../../crates/cmvs-font/src/font.rs#L28) 与 [`crates/cmvs-font/src/font_library.rs`](../../crates/cmvs-font/src/font_library.rs#L19) 将单个字体文件体积硬限制在 **`MAX_FONT = 32 MiB`**。macOS 上部分系统字体合集（如 `PingFang.ttc`）通常超过该上限。
- **启动器对策**：
  - 使用 CoreText（`CTFontManagerCreateFontDescriptorsFromURL`）解析 `.ttf` / `.otf` / `.ttc` 文件内的所有字面（face index、本地化字体族与样式名称），在 UI 中直接提供下拉选择，彻底取代手填索引；
  - 读取文件大小并**提前标示或过滤超过 32 MiB 的字体**，给出清晰提示与替代建议，而非等游戏启动时报错；
  - **macOS**：遍历 [`cmvs-native/src/macos/mod.rs`](../../crates/cmvs-native/src/macos/mod.rs#L12-L22) 列出的系统与用户字体目录，自动筛选合法大小且覆盖中日文字符的字体作为开箱默认推荐，并支持导入外部字体文件；
  - **iOS**：支持从「文件」App 导入字体到应用沙盒（与 Android 相同保留 32 MiB 上限校验），支持**全局默认字体 + 单游戏独立覆写**（便于同时管理 Shift-JIS 日文原版与 GBK/UTF-8 汉化补丁版）；
  - 游戏内切换字体请求接入 `FontRenderer::import_font`，通过 SwiftUI 字体选择器 Sheet 异步回传结果并记录到用户目录 `font-faces/`。

### 4.6 文件系统与存档策略

| 平台 | 游戏接入模式 | 存档存放与迁移 |
|---|---|---|
| **macOS**（非沙盒） | **原地引用目录**（只读访问游戏根目录，不额外复制数 GB 游戏文件；支持拖拽文件夹添加） | 默认存放在 `~/Library/Application Support/org.cmvs.player/Saves/<game-id>/`；提供显式开关「使用游戏目录内 `savedata/`」，启用后调用 `cmvs_host::game_savedata` 一次性无覆盖导入旧存档，便于与 Windows / Linux 互通 |
| **iOS / iPadOS** | **双模式并存**：<br>1. **直接引用**（Security-Scoped Bookmark，零额外空间占用，适合本机「文件」或外接存储）；<br>2. **导入复制**（异步拷贝至 `Documents/Games/<uuid>/`，带进度条与可取消按钮，沿用 [`GameImport.kt`](../../platforms/android/app/src/main/java/org/cmvs/player/GameImport.kt) 的路径与结构校验） | 统一存放在 `Documents/Saves/<game-id>/`；开启 `UIFileSharingEnabled` 与 `LSSupportsOpeningDocumentsInPlace` 后，用户可在系统「文件」App 直接浏览 `Games/` 与 `Saves/`，也支持应用内一键导出/导入 ZIP 备份（含 `font-faces/`） |

### 4.7 输入、手势与游戏内弹窗

- **macOS**：
  - 鼠标左/中/右/侧键映射至 `1/2/3/6/7`，滚轮上下映射至 `4/5`；键盘物理码严格对齐 [`crates/cmvs-desktop/src/game_input.rs`](../../crates/cmvs-desktop/src/game_input.rs#L159-L267) 映射至 CMVS `20..122`（左右 Shift / Ctrl 共享编码、失焦清空防粘连）；
  - 接入 `CursorBitmap` 转换为 `NSCursor`（支持透明区域、hotspot 与脚本控制的自动隐藏）；
  - 应用菜单栏快捷键（`⌘F` 切换全屏、`⌘S` 逻辑帧截图、`⌘W` 保存偏好退出）与引擎键盘事件隔离。
- **iOS / iPadOS**：
  - 主画布由 `cmvs-input::TouchInput` 驱动：严格按 letterbox 视口映射到游戏逻辑坐标，黑边起手与多指冲突不误触，短促点击跨帧保留；
  - **可配置视觉小说手势**（可在全局或单游戏设置中独立开关）：
    - 双指轻点 → 右键（CMVS `3`，唤出菜单 / 隐藏对话框 / 返回）；
    - 双指上下滑动 → 滚轮上 / 下（CMVS `4/5`，打开与翻阅履历）；
    - 三指轻点或屏幕边缘半透明胶囊 → 展开/收起**快捷操作条**（右键、快进长按/锁定 `Ctrl 24`、确认 `Enter 22`、隐藏界面 `Space 26`、一键逻辑帧截图、返回游戏库）；
  - **iPadOS 外接外设**：支持触控板/鼠标悬停移动与右键点击，支持外接物理键盘映射；
  - **手柄支持（后续里程碑）**：通过 `GameController` 框架复用 [`crates/cmvs-console/src/controls.rs`](../../crates/cmvs-console/src/controls.rs) 映射规则。
- **游戏内弹窗**：
  - **存档注释编辑**：原生 SwiftUI Sheet/弹窗，支持日文/中文输入法（IME）、最大字符数限制及目标编码（Shift-JIS / GBK / UTF-8）可编码性实时校验；
  - **消息确认与错误页**：渲染或脚本异常时弹出结构化错误详情页，展示脚本名、PC、步数与一键复制诊断信息。

---

## 5. SwiftUI 界面规划

| 界面模块 | macOS 形态 | iOS / iPadOS 形态 | 核心功能 |
|---|---|---|---|
| **主导航** | `NavigationSplitView` 侧栏 + 菜单栏 | iPhone 使用 `TabView`；iPad 使用 `NavigationSplitView` | 游戏库 / 字体 / 存档 / 设置四大板块 |
| **游戏库** | 网格/列表视图、拖拽导入、右键菜单（打开目录/编辑配置/删除条目） | 卡片列表、右上角「添加游戏」（直接引用 / 导入复制） | 展示游戏名称、引擎标签（CMVS/CVNS）、判定编码、最后游玩时间与缩略图 |
| **单游戏配置** | 右侧 Inspector 或 Sheet | 推入详情页或 Sheet | 编码覆盖、字体与 TTC 字面、Metal/CPU 渲染器、缩放/比例、存档模式（macOS）、`skip_auto_resume`、档案覆盖等诊断参数 |
| **字体管理** | 系统字体自动筛选 + 自定义导入 + 实时预览 | 已导入字体列表 + TTC 字面展开 + 实时预览 | 直观显示文件大小与 `<= 32 MiB` 状态标签、中日文测试句预览 |
| **存档管理** | 游戏存档清单 + 「在 Finder 中显示」+ 导入/导出 ZIP | 游戏存档清单 + 系统分享导入/导出 ZIP | 显示系统档与剧情槽数量、占用空间、安全防覆盖确认 |
| **播放器** | 独立 `WindowGroup`（原生全屏、标题同步、自定义光标） | 全屏横屏 `fullScreenCover`（隐藏 Home 指示条）+ 快捷浮层 | Metal 60fps 渲染、原生评论弹窗、游戏内切字体、结构化错误诊断 |

---

## 6. GitHub Actions CI 与 Release 管线设计

由于开发阶段主要在 Linux 上推进，Apple 平台的编译验证与产物打包完全依托 GitHub Actions：

1. **CI 编译校验（扩展 [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)）**
   - 在 `macos-latest` runner 上新增 `apple` job（与现有 `full` / `macos-intel` / `android` job 并行）：
     1. 安装 Rust 目标：`aarch64-apple-darwin`、`x86_64-apple-darwin`、`aarch64-apple-ios`、`aarch64-apple-ios-sim`；
     2. 安装 `xcodegen`；
     3. 运行 `cargo clippy --locked -p cmvs-apple`（分别覆盖 macOS 与 iOS 目标，`-D warnings`）与 `cargo test --locked -p cmvs-apple`；
     4. 执行 `sh platforms/apple/build.sh xcframework` 生成通用 `CmvsApple.xcframework`；
     5. 执行 `xcodegen generate` 并调用 `xcodebuild` 编译：
        - macOS Universal `.app`（`Release` / `Debug`）；
        - iOS Simulator `arm64` `.app` 并运行 Swift 单元测试；
        - iOS Device `arm64`（`CODE_SIGNING_ALLOWED=NO` 验证真机链接）；
     6. 上传编译产物 artifact（macOS `.app` zip 与 iOS Simulator `.app`），供人工下载验证。
   - 保留 `CMVS_GAME_DIR` / `CMVS_SMOKE_FRAMES` / `CMVS_SMOKE_OUTPUT` 环境变量诊断路径，延续原 `platforms/apple/ios/main.mm` 的模拟器截帧回归能力。

2. **Release 分发（扩展 [`.github/workflows/release.yml`](../../.github/workflows/release.yml)）**
   - 在 tag 发布流中增加 `apple` job：
     - 产出 **macOS Universal `.dmg`（或 `.zip`）**（非沙盒 `.app`）；
     - 产出 **iOS 未签名 `.ipa`**（由 `Release` 构建的 `.app` 封装 `Payload/` 生成，无内置游戏或字体）；
     - 一并纳入 `SHA256SUMS` 并上传至 GitHub Release。

---

## 7. 分阶段实施路线图

### Phase 1：Rust Apple 桥接层（`cmvs-apple`）与构建脚本
- 在根 [`Cargo.toml`](../../Cargo.toml) 注册 `platforms/apple/native`（`cmvs-apple`）。
- 实现 `inspect.rs`、`input.rs`、`backend.rs`、`renderer.rs`、`ffi.rs`、`cmvs_apple.h` 与 `module.modulemap`。
- 更新 [`platforms/apple/build.sh`](../../platforms/apple/build.sh) 支持构建 `CmvsApple.xcframework`。
- **验收标准**：
  - Linux 本机 `cargo test --locked -p cmvs-apple` 全部通过（探测协议、键码/触摸映射、错误缓冲、编码策略）；
  - 仓库全量 `cargo fmt --all --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings` 与 `cargo test --offline` 无回归。

### Phase 2：XcodeGen 工程与 SwiftUI 启动器骨架
- 编写 `platforms/apple/project.yml` 与 `platforms/apple/app/Shared/` 核心数据模型、Swift 封装层（`CmvsBridge`）、游戏库/书签存储、CoreText 字体检查器与启动器 UI。
- 在 `.github/workflows/ci.yml` 接入 `apple` CI job。
- **验收标准**：
  - GitHub Actions `macos-latest` 成功完成 XCFramework 打包、`xcodegen` 生成工程、macOS & iOS 双平台 `xcodebuild` 编译以及 Swift 单元测试。

### Phase 3：Metal / CPU 播放器核心闭环
- 完成 macOS `NSViewRepresentable` 与 iOS `UIViewRepresentable` 的 `CAMetalLayer` 宿主视图与 `displayLink` 帧循环。
- 接入 `cmvs-wgpu` Metal 渲染与可选 CPU 渲染、完整 macOS 键鼠/光标/全屏、iOS 触摸映射、`AVAudioSession` 生命周期、前后台暂停与 `CMVS_SMOKE_*` 诊断入口。
- **验收标准**：
  - macOS `.app` 与 iOS 模拟器能够稳定打开游戏进入标题画面、正常播放音频、缩放/全屏/切后台无崩溃且退出时正确保存设置；
  - 若在 CI 中使用测试夹具运行模拟器冒烟（`CMVS_SMOKE_FRAMES`），可导出非黑屏有效逻辑帧。

### Phase 4：视觉小说交互增强与工具完善
- 实现 iOS 双指右键、双指滑动履历与可折叠快捷操作条（含快进锁定、隐藏对话框、截图、回库），支持 iPadOS 键鼠。
- 完成 SwiftUI 原生存档注释输入 Sheet（含编码预校验）、游戏内字体选择 Sheet、存档 ZIP 导入/导出、最后画面缩略图缓存与结构化错误诊断页。
- 可选接入 `GameController` 手柄支持。
- **验收标准**：完成「启动 → 游玩 → 保存带注释存档 → 退出重启 → 读档继续 → 导出存档备份」全链路验证。

### Phase 5：Release 自动化、文档更新与旧原型清理
- 在 `.github/workflows/release.yml` 增加 macOS `.dmg`/`.zip` 与 iOS 未签名 `.ipa` 发布产物。
- 移除过渡期的旧入口 `platforms/apple/ios/main.mm`。
- 更新 [`platforms/apple/README.md`](../../platforms/apple/README.md)、[`platforms/README.md`](../../platforms/README.md) 与 [`docs/README.md`](../README.md)，记录实机/模拟器验证范围与构建指令。

---

## 8. 关键风险与工程红线

1. **无本地 macOS 开发环境下的反馈周期**：SwiftUI 语法、Metal 表面创建与 CoreText 细节只能在 GitHub Actions 编译或实际设备上验证。应尽量将复杂状态逻辑下沉到 Rust `cmvs-apple` 并在 Linux 本机用单元测试覆盖，Swift 端保持轻量薄壳。
2. **iOS 音频中断恢复**：若 `cpal` CoreAudio 在 iOS 来电/锁屏长时间挂起后出现流卡死，按第 4.4 节预案切换至 `cmvs-audio` `mixer` + `AVAudioEngine` 拉流。
3. **字体体积与内存预算**：严格在启动器 UI 层用 CoreText 预检 `32 MiB` 上限并引导选择合适字重的单字体或小型合集；同时关注 iOS 设备内存限制，遵循 `cmvs-wgpu`（128 MiB 纹理缓存）与 `SessionConfig` 的合理缓存预算。
4. **`CAMetalLayer` 与 `wgpu` 析构时序**：关闭游戏窗口或离开播放器页面时，必须先停帧循环并释放 Rust 侧 `Presenter` / `wgpu::Surface`，再销毁视图对象。
5. **仓库合规红线**：`.gitignore` 必须忽略生成的 `.xcodeproj`、`CmvsApple.xcframework` 及本地构建产物；测试、CI 与 Release 严禁提交任何受版权保护的游戏资产或商业字体文件。
