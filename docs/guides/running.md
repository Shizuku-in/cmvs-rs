# 运行指南

需要 Rust 1.90+ stable。请在仓库根目录执行，示例中的路径请换成实际路径。含空格的路径需加引号。`--offline` 要求依赖已缓存。游戏支持与平台构建见[游戏档案](../games/README.md)和[平台总览](../../platforms/README.md)。

## 桌面运行

```sh
cargo run --offline --release --features desktop --bin cmvs-desktop -- \
  game --encoding shift-jis --font font.ttf --user-data-dir user-data
```

- `--encoding` 必填。使用 `shift-jis`、`gbk`、`utf-8` 或 `auto`；自动判断有歧义时请指定实际编码。
- `--font` 接受 OpenType 字体，TTC 用 `--font-face <索引>` 选面，默认 0。仅调试文字时，可以改用 `--record-text` 记录字形和显示时序。
- `--user-data-dir` 指定游戏目录之外的可写目录，以保存剧情、系统进度和设置。
- 关闭窗口退出。Esc 按游戏绑定处理。

完整选项可用 `cargo run --offline --features desktop --bin cmvs-desktop -- --help` 查看。

## 音频与图形

| 功能 | 构建特性 | 运行参数 |
|---|---|---|
| 原生音频 | `desktop,audio-output` | `--audio native` |
| GPU 渲染 | `wgpu-renderer` | `--renderer wgpu` |
| 原生音频与 GPU | `wgpu-renderer,audio-output` | `--renderer wgpu --audio native` |

默认记录音频请求并使用 CPU 渲染。设备要求和后端限制见[音频输出](../backends/audio-output.md)与 [wgpu 后端](../backends/wgpu.md)。

## 独立用户目录

| 引擎 | 系统档 | 设置 |
|---|---|---|
| CMVS | `system.dat` | `cmvs.cfg` |
| CVNS | `save000.dat` | `cvns.cfg` |

系统档和设置优先读取用户目录，缺失时回退到游戏原文件。指定用户目录后，剧情槽也从该目录读取，设置在正常退出时保存。迁移已选字体时请一并复制 `font-faces/`。同一用户目录同时只支持一个会话。备份、恢复及提交规则见[系统存档](../formats/system-save-format.md#独立用户目录)。

## 剧情存档

从支持的游戏菜单或快捷键存读档。导入原版存档时，可将系统档和所需剧情槽复制到用户目录，再从游戏菜单载入。若系统档启用了启动自动读档，也需复制对应的剧情槽。

CVNS 系统档占用 `save000.dat`，首个剧情槽使用 `save001.dat`。导入原版 CVNS 剧情快照目前限于 `cvns-20090203` 的 `CSP1` v3。备份槽、格式和恢复边界见[存档格式](../formats/save-format.md)，实测范围见[游戏档案](../games/README.md)。

## 诊断与检查

| 选项 | 用途 |
|---|---|
| `--exit-after <秒>` | 到时间后正常退出 |
| `--save-frame <新文件.png>` | 导出最后成功呈现的完整逻辑帧 |
| `--continue-without-snapshot` | 仅应用快照点的实时效果，停用剧情存档 |
| `--skip-auto-resume` | 临时跳过启动自动读档，要求已识别游戏及其自动读档条件 |
| `--profile <档案ID或游戏别名>` | 显式指定引擎档案 |
| `--block-presumed` / `--block-unreviewed` | 分别阻止仅有推定证据或尚未复核的命令 |

默认允许推定和未复核命令，缺失及空操作命令始终停止。命令状态见[引擎档案](../design/engine-profiles.md)，资源检查、trace 和探针见 [CLI 指南](cli.md)。

## 游戏内对话框

存档评论、CVNS 退出确认和取色器共用游戏内面板。评论可在桌面直接输入，Web、移动端和主机可选择输入框后使用平台键盘。评论支持全角和半角字符，长度上限由游戏脚本决定。选「OK」提交，选「Cancel」保留原评论。对话框打开时脚本暂停。

macOS 构建与系统字体选择见 [Apple players](../../platforms/apple/README.md)及[字体后端](../backends/font-output.md)。
