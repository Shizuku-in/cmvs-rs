# cmvs-native

桌面宿主使用的操作系统服务层，通过统一 Rust 接口隔离平台差异。目前提供系统字体选择对话框与系统字体目录查询，不承担 VM 或游戏命令语义。

接口与实现：

- `choose_font`：以宿主窗口为所有者打开字体对话框；取消返回 `Ok(None)`，未实现的平台返回 `NativeError::Unsupported`。
- `font_directories`：返回系统字体目录；当前未实现的平台返回空列表。
- Windows 实现在独立模块中；macOS、Linux、iOS、Android 和 wasm 尚未实现这些服务。

本包依赖 `raw-window-handle`，仅在 Windows 引入 `windows-sys`。它是工作区唯一允许项目自身 FFI 的包：默认拒绝 `unsafe`，只在注明 `SAFETY` 的系统调用处局部放行。新增平台服务应在这里实现，调用方保持统一接口。

从仓库根目录检查：

```bash
cargo test --offline -p cmvs-native
cargo clippy --offline -p cmvs-native --all-targets -- -D warnings
```

自动测试覆盖接口和回退行为，字体对话框交互仍需原生人工验证。

详见[宿主契约](../../docs/backends/platform-host.md)、[字体输出](../../docs/backends/font-output.md)和[Workspace 边界](../../docs/workspace.md)。
