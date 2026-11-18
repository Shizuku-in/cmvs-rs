# 运行指南

从项目根目录执行，需要 Rust 1.90+ stable；`--offline` 要求依赖已缓存。以下命令中的游戏、字体和用户目录请替换为实际路径。游戏支持范围见[游戏档案](../games/README.md)，平台依赖见[平台总览](../../platforms/README.md)。

## 桌面运行

```sh
cargo run --offline --release --features desktop --bin cmvs-desktop -- "<游戏目录>" --encoding shift-jis --user-data-dir "<独立用户目录>" --audio record --font "<字体文件>"
```

默认构建剧情快照。关闭窗口可退出，Esc 按游戏绑定处理。

| 选项 | 用途 |
|---|---|
| `--encoding shift-jis` | 选择文本编码，也可用 `gbk` / `utf-8`；标题、引擎扫描和字体使用同一编码 |
| `--encoding auto` | 从启动及归档脚本推测编码，证据不足时需手动指定 |
| `--font <字体>` | 提供覆盖游戏字符的 OpenType 字体；TTC 用 `--font-face <索引>` 选面，默认 0 |
| `--audio record` | 记录音频请求，不打开设备 |
| `--exit-after <秒>` | 有界运行后正常退出 |
| `--save-frame <新文件.png>` | 导出最后成功呈现的完整逻辑帧，拒绝覆盖已有文件 |

须指定编码。文字输出需要 `--font` 或 `--record-text`，后者只记录字形。

## 原生音频

启用 `--features desktop,audio-output` 并使用 `--audio native`。解码与设备要求见[音频输出](../backends/audio-output.md)。

## 独立用户目录

`--user-data-dir` 指定原游戏目录之外的可写目录。CMVS 使用 `system.dat` 和 `cmvs.cfg`，CVNS 使用 `save000.dat` 和 `cvns.cfg`。用户文件优先，缺失时读取原游戏中的对应文件。剧情槽从用户目录读取。

每个用户目录同一时间只能打开一次。已有文件损坏或只剩备份时会报错，关闭会话后可手动恢复。首次保存和文件替换规则见[系统存档](../formats/system-save-format.md#独立用户目录)。

## 剧情存档

在支持的游戏中，从游戏菜单或快捷键存读档。普通槽覆盖前会备份到 `save999.dat`，读档前可把当前快照备份到 `save998.dat`。启用了启动自动恢复的用户目录须包含对应槽。

导入原版存档时，把系统存档和需要的剧情槽复制到用户目录，再从游戏菜单载入。兼容性取决于游戏构建和快照内容。CVNS 当前支持 `cvns-20090203` 的原版 `CSP1`。各游戏的实测范围见[游戏档案](../games/README.md)及[存档格式](../formats/save-format.md)。

## 设置保存与重启

指定用户目录后，桌面在正常退出时保存设置，含诊断倒计时退出。无窗口调用方需调用 `Runtime::save_preferences` 或 `Player::save_preferences`。

配置保留未知键和原始文本。显示偏好使用 `[CMVS_RS_DESKTOP]`，比例滤镜沿用 `[GRAPHIC_CONFIG]` 的 `ASPECT_RATIO_FILTER`（0 线性、1 点采样）。

迁移已选字体时一并复制 `font-faces/`。缺字时回退到 `--font` 提供的字体，详见[字体后端](../backends/font-output.md)。

INI 和系统存档分别替换并备份，后一步失败时前一步可能已保存。退出保存系统数据要求启动时已执行读取命令（含文件缺失）或已加载存档模板。

## 选择图形后端

桌面默认 `--renderer cpu`。启用 `--features wgpu-renderer` 后可传 `--renderer wgpu`，初始化失败明确报错。CPU / GPU 共用绘制队列和帧边界，截图与限制见 [wgpu 后端](../backends/wgpu.md)。

## macOS

需要 Rust stable 与 Xcode Command Line Tools。本机默认 target 可构建桌面版本；原生音频使用 `desktop,audio-output`，GPU 使用 `wgpu-renderer,audio-output` 与 `--renderer wgpu`。Apple Silicon / Intel、iOS 构建入口和验证范围见 [Apple players](../../platforms/apple/README.md)。

游戏内字体选择使用系统 osascript / AppKit，独立对话框期间 VM 暂停，取消保留当前设置。字体后端仍会验证所选格式和字形；不提供 `.app` 打包、签名及公证。

## 存档评论编辑框

Windows 使用内置模态窗口；Linux 使用 Zenity（需安装 `zenity`）；macOS 使用系统 osascript 输入对话框。X11 可传递父窗口，Wayland 和 macOS 对话框独立显示，期间 VM 暂停。

确认后遵守所选构建的全角过滤、空输入回退、游戏编码和字段容量，取消不修改评论。Linux / macOS 按字符边界截取至脚本容量并保留前后空格；后端启动失败明确报错。

## 诊断与检查

| 选项 | 含义与限制 |
|---|---|
| `--continue-without-snapshot` | 仅应用快照点的实时效果，不构建镜像，此后剧情存档不可用 |
| `--skip-auto-resume` | 跳过已登记游戏的启动自动读档策略，磁盘存档不变；未登记策略的游戏拒绝 |
| `--profile <id>` | 显式选择引擎档案；通常使用内置身份识别即可 |
| `--block-unreviewed` | 阻止未复核命令 |
| `--block-presumed` | 阻止仅有推定证据的命令，与上一项独立 |

默认允许未复核和推定命令，首次使用会输出诊断；缺失及空操作命令始终停止。运行通过不提高证据等级，见[引擎档案](../design/engine-profiles.md)。资源检查、trace 与探针命令见[CLI 指南](cli.md)。
