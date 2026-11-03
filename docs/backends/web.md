# WebAssembly 后端

构建、运行与当前边界见[浏览器运行器说明](../../platforms/web/README.md)。

`platforms/web/native` 是独立 `cdylib`，依赖 core、共享字体光栅化、禁用原生设备 feature 的 cmvs-audio、wasm-bindgen 与 self_cell；wasm32 另外连接 cmvs-wgpu、web-sys 与 wasm-bindgen-futures。脚本与借用它的 `Startup` 由 self_cell 管理，销毁运行器会释放状态，不泄漏静态脚本。

核心的 `filesystem` 模块在原生目标转发标准文件系统调用，在 wasm32 目标导入 Worker 提供的文件接口。`File` 实现 `Read + Seek`，CPZ、影片及档案哈希继续使用现有读取器；宿主以 FileReaderSync 读取 Blob 片段，每次最多 1 MiB，无需将整个游戏包复制进 wasm 内存。路径解析及资源校验仍由核心执行，浏览器侧额外限制游戏根只读、用户目录可写并拒绝路径穿越。原生核心没有新增第三方依赖，wasm32 核心使用 wasm-bindgen 连接宿主。

Worker 运行有限步数的 VM 切片，在 `Yield` 后推进时间、渲染完整帧；CPU 路径发布 RGBA 到页面 Canvas，WebGPU 路径在 Worker 的 OffscreenCanvas 直接呈现。主页面负责物理输入和调度。输入仅在下一引擎帧开始时锁存。显示能力描述虚拟设备，浏览器窗口和全屏权限由页面按钮处理。

存档文件首先完成内存内的临时文件替换及备份，Worker 在帧边界将完整用户目录快照提交为一个 IndexedDB 事务。Web Locks 限制同一存档命名空间只有一个 Worker；不把内存内的 `sync_all` 当作已落盘保证。事务失败会报错停止，不能认领为持久化成功。停止操作和游戏正常退出等待持久化后才通知页面。

原生文件系统行为由已有资源、运行器、配置与存档测试验证；浏览器适配与 wasm 执行使用合成回归。尚未验证真实游戏的浏览器端交互、完整路线与设备性能。

此前按需浏览器检查在 Windows 无头 Edge 的实际模块 Worker 中通过：CPU/GPU 四帧的文字、场景与即时截图逐通道差异不超过 2，预算切片不发布画面。实际页面通过 WebGPU→CPU→WebGPU 重启、画布尺寸与呈现像素、停止保存，以及截图运行期间排队导出/停止的检查。Web 端自动测试仅保留音频回归与一个实际 WASM smoke，WebGPU 对照改为按需人工验证，不保留独立浏览器测试入口。原生 GPU 合成回归与 WASM Clippy 通过；这些结果不覆盖真实游戏路线、其他浏览器、移动设备或设备丢失恢复。

## 浏览器音频

`cmvs-audio::VorbisProgram` 从原生设备代码独立出来，原生默认 feature 继续提供 rodio 输出；Web 宿主仅使用无设备依赖的流式解码器。浏览器后端复用资源读取、语音压缩及语音压低 BGM 状态，16 个设备槽分别管理解码器、准备/启动状态和代次。

Worker 按主页面反馈的源消费进度提供小段交错 PCM；传输前复制 wasm 内存视图，再转移独立缓冲区。页面以 Web Audio 调度 AudioBufferSourceNode，以独立增益节点处理音量与渐变。时钟累计实际排程的源帧，排除初始等待和欠载空白；通道代次隔离延迟反馈，停止、换音、渐隐结束均取消排队音源。静音继续消费，不重启或暂停解码器。后台定时器节流可能导致欠载；未进行设备延迟校准或实际听音验证。

音频回归包括共享解码与循环、Web Audio 调度时钟、静音与渐变、旧通道清理，以及实际 wasm 的 BGM/音效/语音与加速压缩解码、流式补充和退出。浏览器完整游戏路线及扬声器输出仍待验证。
