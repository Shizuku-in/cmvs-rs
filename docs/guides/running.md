# 运行指南

命令从项目根目录执行，需要 Rust 1.89+ stable。`--offline` 要求依赖已缓存。以下桌面参数组合目前以[《アマツツミ》PC 样本](../games/amatsutsumi/README.md)验证，游戏路径和字体路径请替换为本机路径。

## 桌面预览

```powershell
cargo run --offline --release --features desktop --bin cmvs-desktop -- "E:/Download/AMATSUTSUMI" --encoding shift-jis --user-data-dir "local/amatsutsumi/user-data" --audio record --font "C:/Windows/Fonts/msgothic.ttc"
```

默认在 0x2080/0x22b2 构建引擎存档快照；存档、读档与系统存档都写入 `--user-data-dir`，原游戏目录保持只读。快照点上存在尚无存档格式的状态时以错误停止，不写不完整镜像，范围见[剧情存档](#剧情存档)。

| 选项 | 含义与限制 |
|---|---|
| `--encoding shift-jis` | 显式选择文本编码；另可选 `gbk`、`utf-8`，不能据此推断对应游戏已兼容 |
| `--user-data-dir <目录>` | 独立用户目录：系统存档、剧情槽 `saveNNN.dat` 的读写位置，见下文 |
| `--continue-without-snapshot` | 研究用：不构建快照，只应用已核对的实时影响并继续；此后剧情存档不可用 |
| `--skip-auto-resume` | 研究用：首次系统存档读取成功后清零游戏记录中登记的内存整数（本作为 2198），跳过启动自动读档；只对记录了该策略的游戏可用（目前仅本作），磁盘存档保持不变 |
| `--profile <id>` | 研究用：显式选择引擎档案。默认按启动脚本、脚本包与 exe 的哈希自动识别，未登记的游戏会拒绝启动 |
| `--allow-unreviewed` | 研究用：用基准语义放行当前档案中未复核的命令，每个命令首次放行时打印 `# UNREVIEWED`；不存在和空操作的命令仍会停止 |
| `--font <字体>` | 用桌面 OpenType 后端显示文字；字体路径由用户提供，与原版 GDI 有像素差异 |
| `--audio record` | 仅记录音频请求，不发声 |

未指定字体且未启用 `--record-text` 时保留文字输出屏障；`--record-text` 只记录字形。关闭窗口可退出；活动游戏中的 Esc 按游戏动作绑定处理。`--exit-after <秒>` 可做有界运行，`--save-frame <新文件.png>` 可导出退出时的帧。

## 原生音频

```powershell
cargo run --offline --release --features desktop,audio-output --bin cmvs-desktop -- "E:/Download/AMATSUTSUMI" --encoding shift-jis --user-data-dir "local/amatsutsumi/user-data" --audio native --font "C:/Windows/Fonts/msgothic.ttc"
```

普通 Windows 桌面权限下已验证品牌音效和标题 BGM；受限沙箱中相同构建曾以 `0xc0000008` 退出。设备检查、源消费时钟与验证边界见[原生音频](../backends/audio-output.md)。其他平台和完整剧情音轨仍需独立实测。

## 独立用户目录

`--user-data-dir` 的目录必须位于原游戏根之外。用户 `system.dat` 优先，缺失时才从原游戏只读引入 CSS1 模板；支持重复保存、`system.bak` 与会话锁。损坏文件明确报错，不自动恢复备份。无 CSS1 模板的首次保存尚不支持。

单次实验可改用 `--system-save-output "local/amatsutsumi/completed-system.dat"`，只允许游戏目录之外的新文件、拒绝覆盖。该选项与 `--user-data-dir` 互斥。不配置这两种方式时保留 `0x2129` 系统存档写入屏障，剧情存档报错。细节见 [CSS1](../formats/system-save-format.md)。

## 剧情存档

脚本命令 `0x22b6` 写 `saveNNN.dat`、`0x22b5` 读档，与原版同样：覆盖 0..949 号槽前把旧文件保存为 `save999.dat`，读 0..949 号槽前把当前快照保存为 `save998.dat`，每次写槽同时写 `system.dat`。没有原版的覆盖确认对话框。标签时间使用 UTC（标准库不含本地时区），缩略图来自核心渲染帧，不含桌面字体绘制的文字。

目前桌面只能经启动自动读档进入剧情存档：本机 `system.dat` 记录了自动读档时（本作整数 2198 非零），启动后恢复对应槽，用户目录需要包含该槽文件。游戏内工具栏与存读档菜单依赖尚未实现的自动回调调度，点击只推进文字，因此还不能从界面存档或选择槽读档；核心 API `Startup::save_slot` / `load_slot` 与外部测试使用同一路径。快照点存在下列状态时明确报错：按钮/列表组、辅助覆盖层、旋转轨道、跨脚本调用中或已启用存档回调 30。读档后遮罩转场从头开始，条带转场按原版直接成为完成状态。格式与读档效果见 [CSV2](../formats/save-format.md) 和[引擎快照](../formats/snapshot-format.md)。

## 研究与检查

脚本 trace、资源检查和导出示例见 [CLI 指南](cli.md)。桌面、输入和字体的具体支持分支见[后端文档](../README.md)，当前结局路线与鉴赏停点见[游戏档案](../games/amatsutsumi/README.md)。原始文件保持只读，真实输出放游戏目录外并避免进入 Git。
