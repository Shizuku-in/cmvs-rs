# 运行指南

从项目根目录执行，需要 Rust 1.89+ stable；`--offline` 要求依赖已缓存。以下命令中的游戏、字体和用户目录请替换为实际路径。游戏支持范围见[游戏档案](../games/README.md)，平台依赖见[平台总览](../../platforms/README.md)。

## 桌面运行

```sh
cargo run --offline --release --features desktop --bin cmvs-desktop -- "<游戏目录>" --encoding shift-jis --user-data-dir "<独立用户目录>" --audio record --font "<字体文件>"
```

默认构建剧情存档快照；独立用户目录保存系统存档、剧情槽和设置，原游戏保持只读。未知游戏、未实现命令、资源或快照状态会明确报错。关闭窗口可退出；活动游戏中的 Esc 按游戏绑定处理。

| 选项 | 用途 |
|---|---|
| `--encoding shift-jis` | 选择文本编码，也可用 `gbk` / `utf-8`；标题、引擎扫描和字体使用同一编码 |
| `--encoding auto` | 从启动脚本文本区的完整字符串推测编码；纯 ASCII、证据不足或歧义时要求手动指定 |
| `--font <字体>` | 提供覆盖游戏字符的 OpenType 字体；TTC 用 `--font-face <索引>` 选面，默认 0 |
| `--audio record` | 记录音频请求，不打开设备 |
| `--exit-after <秒>` | 有界运行后正常退出 |
| `--save-frame <新文件.png>` | 导出最后成功呈现的完整逻辑帧，拒绝覆盖已有文件 |

必须选择编码；自动检测不保证适用于混合编码或任意补丁。未指定字体且未启用 `--record-text` 时停在文字输出；`--record-text` 只生成字形记录，与 `--font` 互斥。

## 原生音频

在上述命令中启用 `--features desktop,audio-output`，并将 `--audio record` 改为 `--audio native`。不支持的设备或解码错误直接报错。解码、音轨时钟和设备检查见[音频输出](../backends/audio-output.md)；设备消费通过不代表人工听感或同步质量通过。

## 独立用户目录

`--user-data-dir` 必须位于原游戏根之外。首次优先读取用户的 `system.dat` 和 `cmvs.cfg`，缺失时才使用原游戏只读模板；无系统存档模板时，可在初始化完成后从完整持久状态首次创建。原游戏缺少 `save` 目录时按无存档启动，不创建原游戏目录。

目录由会话锁保护，不能同时打开。保存使用临时文件替换及备份；损坏文件、缺少主文件但存在备份等情况明确报错，需关闭会话后自行恢复，不静默覆盖。系统存储协议见 [CSS1](../formats/system-save-format.md)。

## 剧情存档

在支持的游戏中，使用游戏内快存 / 快读、存读档页或相应快捷键。写入普通槽前保留旧槽为 `save999.dat`，读取前可将当前快照保留为 `save998.dat`；写槽同时保存系统数据。缩略图来自核心渲染，不含字体后端字形；时间标签使用 UTC。

已记录启动自动恢复的用户目录需要包含对应剧情槽。快照遇到未知记录或未支持状态会报错，不写不完整镜像。换图进行中可保存，恢复后显示完整新图；遮罩转场重新开始，条带按原版成为结束状态。容器与恢复细节见 [CSV2](../formats/save-format.md) 和[引擎快照](../formats/snapshot-format.md)，各游戏的保存验证范围见其档案。

## 设置保存与重启

提供用户目录时，正常退出（含诊断倒计时退出）保存已实现的用户设置；VM 错误退出不自动保存。无窗口调用方需显式调用 `Runtime::save_preferences` 或 `Player::save_preferences`，析构不执行可能失败的保存。

配置保留未知键和原始文本。已支持的持久设置包括声音开关 / 音量、语音压低 BGM、文字样式 / 速度 / 显现、自动送页、未读停止、后台运行、效果、保存确认、光标与显示偏好。显示偏好使用 `[CMVS_RS_DESKTOP]`；绘制滤镜沿用 `[GRAPHIC_CONFIG]` 的 `ASPECT_RATIO_FILTER`（0 线性、1 点采样）。

字体选择使用 Unicode 名称，`font-faces/` 保存完整名称，固定长度配置 / 存档字段保留稳定标识；迁移时须保留该目录。所选字体缺字时按字符回退到 `--font` 提供的字体。名称映射损坏或字体无法加载时回退默认面，详见[字体后端](../backends/font-output.md)。

INI 和 CSS1 分别替换并备份，不是跨文件事务；后一步失败时，前一步可能已保存。退出保存 CSS1 须已完成启动系统存档读取（缺失也算完成）或已有模板，避免初始化前退出生成默认数据。配置最多 1 MiB，拒绝 NUL / UTF-16。未来窗口尺寸只在当前会话保留。

## 选择图形后端

桌面默认 `--renderer cpu`。启用 `--features wgpu-renderer` 后可传 `--renderer wgpu`，初始化失败明确报错。CPU / GPU 共用绘制队列和帧边界，截图与限制见 [wgpu 后端](../backends/wgpu.md)。

## CHRONOCLOCK运行

已登记的《クロノクロック》可用上面的桌面命令运行，无需使用禁用存档的无快照模式。正常剧情采用文字 v2 快照；鉴赏缩略图列表需鼠标，侧栏及剧情键盘输入另有验证。路线、原生操作及特殊快照限制见[游戏档案](../games/chronoclock/README.md)。

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
