# cmvs-native

桌面宿主使用的操作系统服务层，通过统一 Rust 接口隔离平台差异。目前提供系统字体选择对话框与系统字体目录查询，不承担 VM 或游戏命令语义。

接口与实现：

- `choose_font`：以宿主窗口为所有者打开字体对话框；取消返回 `Ok(None)`，未实现的平台返回 `NativeError::Unsupported`。
- `font_directories`：返回系统字体目录；当前未实现的平台返回空列表。
- Windows、Linux 实现按平台目录隔离，统一接口与 `cfg` 分派留在 `src/lib.rs`。Linux 通过 Fontconfig 的 `fc-list` 发现字体目录，通过 Zenity 列表选择支持日文且覆盖可打印ASCII的字体族；需要安装 `fontconfig`、`zenity` 和日文字体。取消不改变当前字体，工具缺失或失败明确报错。Wayland 下选择器为独立窗口；X11 下附着于宿主窗口。macOS、iOS、Android 和 wasm 尚未实现这些服务。

本包依赖 `raw-window-handle`，仅在 Windows 引入 `windows-sys`。它是工作区唯一允许项目自身 FFI 的包：默认拒绝 `unsafe`，只在注明 `SAFETY` 的系统调用处局部放行。新增平台服务应在这里实现，调用方保持统一接口。

从仓库根目录检查：

```bash
cargo test --offline -p cmvs-native
cargo clippy --offline -p cmvs-native --all-targets -- -D warnings
```

自动测试覆盖接口和回退行为，字体对话框交互仍需原生人工验证。

详见[宿主契约](../../docs/backends/platform-host.md)、[字体输出](../../docs/backends/font-output.md)和[Workspace 边界](../../docs/workspace.md)。

## 目录约定

```text
src/
├── lib.rs                  # 公共接口、错误类型及平台分派
├── windows/                # Windows 系统服务
│   ├── mod.rs
│   └── font_dialog.rs
├── linux/                  # Linux 系统服务
│   ├── mod.rs
│   ├── font_dialog.rs      # Zenity 字体选择
│   └── fontconfig.rs       # Fontconfig 字体发现
└── unsupported/            # 尚未实现的平台统一回退
    └── mod.rs
```

新增 macOS、Android 等平台时建立对应平台目录，在 `lib.rs` 中显式分派；不把其他系统的实现放进 `windows/`。本次目录调整不扩展 Linux 实现的目标系统范围，也不改变字体选择行为。
