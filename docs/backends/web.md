# WebAssembly 后端

构建、运行与当前边界见[浏览器运行器说明](../../platforms/web/README.md)。

`platforms/web/native` 是独立 `cdylib`，依赖 core、共享字体光栅化、wasm-bindgen 与 self_cell。脚本与借用它的 `Startup` 由 self_cell 管理，销毁运行器会释放状态，不泄漏静态脚本。

核心的 `filesystem` 模块在原生目标转发标准文件系统调用，在 wasm32 目标导入 Worker 提供的文件接口。`File` 实现 `Read + Seek`，CPZ、影片及档案哈希继续使用现有读取器；宿主以 FileReaderSync 读取 Blob 片段，每次最多 1 MiB，无需将整个游戏包复制进 wasm 内存。路径解析及资源校验仍由核心执行，浏览器侧额外限制游戏根只读、用户目录可写并拒绝路径穿越。原生核心没有新增第三方依赖，wasm32 核心使用 wasm-bindgen 连接宿主。

Worker 运行有限步数的 VM 切片，在 `Yield` 后推进时间、渲染并发布完整 RGBA 帧；主页面负责 Canvas、物理输入和调度。输入仅在下一引擎帧开始时锁存。显示能力描述虚拟设备，浏览器窗口和全屏权限由页面按钮处理。

存档文件首先完成内存内的临时文件替换及备份，Worker 在帧边界将完整用户目录快照提交为一个 IndexedDB 事务。Web Locks 限制同一存档命名空间只有一个 Worker；不把内存内的 `sync_all` 当作已落盘保证。事务失败会报错停止，不能认领为持久化成功。停止操作和游戏正常退出等待持久化后才通知页面。

原生文件系统行为由已有资源、运行器、配置与存档测试验证；浏览器适配与 wasm 执行使用合成回归。尚未验证真实游戏的浏览器端交互、完整路线与设备性能。
