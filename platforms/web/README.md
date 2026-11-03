# 浏览器运行器

这是复用现有 CMVS VM、软件渲染与字体后端的 wasm 预览入口。游戏目录由用户选择，文件不上传；资源包由独立 Worker 按需读取片段。引擎档案仍按现有数据哈希识别，不会默认选择未知游戏。

## 构建与启动

在仓库根目录执行：

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130 --locked
```

Windows：

```powershell
./platforms/web/build.ps1
python -m http.server 8000 --bind 127.0.0.1 --directory platforms/web
```

Linux / macOS：

```sh
sh platforms/web/build.sh
python3 -m http.server 8000 --bind 127.0.0.1 --directory platforms/web
```

打开 <http://localhost:8000>，选择包含 `data/pack` 的游戏根目录和可显示日文的 TTF/OTF/TTC 字体，然后启动。TTC 可指定字体索引。生成文件位于忽略的 `platforms/web/pkg`，部署时将该目录和 HTML/MJS 文件一起提供；服务器须正确提供 `.wasm`（`application/wasm`）和 JavaScript MIME 类型。无需 npm 依赖、跨源隔离或外部 CDN。

需要支持模块 Worker、FileReaderSync、目录选择、IndexedDB 和 Web Locks 的现代浏览器；使用 localhost 或 HTTPS。当前浏览器兼容性与真实游戏交互仍待人工验证。

## 输入与存档

- 鼠标左右键、滚轮、触摸点击及键盘输入转为核心输入码，动作绑定沿用游戏 `key.cfg`；缺失配置时不猜测绑定。F1–F12 同时接入功能键报告。
- 输入仅在引擎帧边界更新；预算切片不消耗下一帧输入，也不显示尚未完成的画面。失焦取消输入。
- 游戏数据只读。系统存档、剧情槽与配置写入独立内存目录，再以事务保存到当前源的 IndexedDB。存档按档案与输入证据隔离；同一游戏通过 Web Locks 限制为一个会话。
- 使用“停止”保存设置并等待存档事务提交。浏览器关闭前最近尚未提交的变更不保证保留；清理站点数据会删除本地存档。可导出 JSON 备份，并在下一次启动时显式导入；导入会替换本游戏的浏览器存档。更换域名或端口后需导入备份。

## 范围与验证

当前支持软件场景与文字呈现、已支持的 CMV 解码和核心存读档路径。音频采用明确的静音预览后端，加载结果与记录后端一致，未模拟真实播放时钟。MGV 解码、系统字体搜索、字体选择对话框与原生光标尚未接入。脚本的显示设置作用于虚拟设备；浏览器全屏由用户点击页面按钮触发。

未知格式、未实现指令与已有核心限制继续显式报错；这不是完整 CMVS 兼容性声明。当前通过 native/wasm Clippy、JS 文件系统回归和实际 wasm 合成执行验证（资源包读取、预算切片、完整帧、CSS1 写入与备份、配置保存、重启回读、退出）。尚未验证真实游戏的浏览器完整路线、IndexedDB 配额及移动设备性能。

```sh
cargo clippy --locked -p cmvs-web --target wasm32-unknown-unknown -- -D warnings
node --test platforms/web/filesystem.test.mjs
node platforms/web/smoke.mjs
```

`smoke.mjs` 在构建后运行，需要字体文件；可设置 `CMVS_TEST_FONT`，也会尝试 Windows Gothic 和 Linux DejaVu 默认字体。测试仅使用已有合成 CPZ fixture 与生成的 PS2A，不包含游戏素材。
