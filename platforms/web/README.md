# 浏览器运行器

这是复用现有 CMVS VM、CPU/WebGPU 渲染、字体与 Vorbis 解码器的 wasm 预览入口。游戏目录由用户选择，文件不上传；资源包由独立 Worker 按需读取片段。引擎档案仍按现有数据哈希识别，不会默认选择未知游戏。

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

需要支持 Web Audio、模块 Worker、FileReaderSync、目录选择、IndexedDB 和 Web Locks 的现代浏览器；使用 localhost 或 HTTPS。当前浏览器兼容性与真实游戏交互仍待人工验证。

“启动选项与当前限制”中的渲染后端可选择 CPU（默认）或 WebGPU。WebGPU 使用 Worker 的 OffscreenCanvas，复用 `cmvs-wgpu` 的场景合成、纹理缓存、文字合成与影片缩放；普通帧直接呈现，不读回或向页面传输 RGBA。字形光栅化、资源与影片解码仍在 CPU。需要浏览器在 Worker 中支持 WebGPU；适配器、设备或初始化失败会明确报错，可手动切换 CPU 后重新启动。

即时截图 `0x203e` 按需异步读取最近成功呈现的逻辑帧，映射期间保留待执行指令，完成后继续；不采集尚未完成的场景。停止和导出操作等待当前运行片段释放 WASM 借用，输入与音频时钟反馈仍可更新。延迟截图 `0x2304` 沿用核心 CPU 路径及其已有边界。停止后重新启动会创建新画布，支持 CPU/WebGPU 之间切换。

## 输入与存档

- 鼠标左右键、滚轮、触摸点击及键盘输入转为核心输入码，动作绑定沿用游戏 `key.cfg`；缺失配置时不猜测绑定。F1–F12 同时接入功能键报告。
- 输入仅在引擎帧边界更新；预算切片不消耗下一帧输入，也不显示尚未完成的画面。失焦取消输入。
- 游戏数据只读。系统存档、剧情槽与配置写入独立内存目录，再以事务保存到当前源的 IndexedDB。存档按档案与输入证据隔离；同一游戏通过 Web Locks 限制为一个会话。
- 使用“停止”保存设置并等待存档事务提交。浏览器关闭前最近尚未提交的变更不保证保留；清理站点数据会删除本地存档。可导出 JSON 备份，并在下一次启动时显式导入；导入会替换本游戏的浏览器存档。更换域名或端口后需导入备份。

## 范围与验证

当前支持 CPU/WebGPU 场景与文字呈现、已支持的 CMV 解码和核心存读档路径。音频通过 Web Audio 输出 BGM、语音、音效及 CMV 内嵌 Vorbis 音轨，支持前奏与循环段、准备后启动、音量、淡入淡出、语音压低 BGM 与单声道加速压缩。启动按钮启用浏览器音频；解码器按消费进度补充约 250～350 ms 的 PCM，不预解码整首曲目。播放位置来自已排程音频的消费时间，扣除启动等待与欠载空白，不宣称为校准后的扬声器时间。MGV 解码、系统字体搜索、字体选择对话框与原生光标尚未接入。脚本的显示设置作用于虚拟设备；浏览器全屏由用户点击页面按钮触发。

未知格式、未实现指令与已有核心限制继续显式报错；这不是完整 CMVS 兼容性声明。当前通过 native/wasm Clippy、音频回归和实际 wasm 合成执行验证（资源包读取、预算切片、完整帧、CSS1 写入与备份、配置保存、重启回读、退出）。音频调度通过合成回归，wasm smoke 覆盖 BGM/音效/普通及加速语音的实际 Vorbis 解码、队列上限、补充与退出清理。尚未进行浏览器设备听音，未验证真实游戏的浏览器完整路线、IndexedDB 配额及移动设备性能。

```sh
cargo clippy --locked -p cmvs-web --target wasm32-unknown-unknown -- -D warnings
node --test platforms/web/audio.test.mjs
node platforms/web/smoke.mjs
```

`smoke.mjs` 在构建后运行，需要字体文件；可设置 `CMVS_TEST_FONT`，也会尝试 Windows Gothic 和 Linux DejaVu 默认字体。测试仅使用已有合成 CPZ fixture 与生成的 PS2A，不包含游戏素材。

Web 端自动测试仅保留音频回归与一个实际 WASM smoke。WebGPU 画面对比、即时截图与后端切换改为按需人工验证；自动 smoke 不覆盖浏览器 GPU 行为。
