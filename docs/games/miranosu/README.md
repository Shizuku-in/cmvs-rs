# 《未来ノスタルジア》测试档案

## 样本身份

| 字段 | 内容 |
|---|---|
| 游戏标识 | `miranosu` |
| 本地样本目录 | `2011-09-02_MIRANOSU` |
| 英文名称 | Mirai Nostalgia |
| 引擎 / 脚本自报版本 | `cmvs64[ver.2.02]` / `ps2[ver.1.00]` |
| 发行版本与补丁 | 未确认；目录日期仅作为样本标签 |
| 平台与架构 | Windows PC；x86 + x64 |
| 文本编码 | 未独立确认 |
| 引擎 profile | `cmvs-20110525`，基准 `cmvs-20160201` |
| 本地封包签名 | CPZ5（9 个包） |

## 文件身份

| 文件 | 架构 / image base 或签名 | SHA-256 |
|---|---|---|
| `cmvs32.exe` | x86 / `0x400000` | `78c2283990f3fec167a8097e5e56304edbad7e94d5367eae9e3f5cfa56b4cc4e` |
| `cmvs64.exe` | x64 / `0x140000000` | `9870e6d5cbd5242b9c686aae20fe10697e87328a66d322cfdb2c80021cb645c5` |
| `data/pack/start.ps2` | `PS2A` | `2e8c7261ea801f59c9e80b9cdbde89832c0a65f7b622e2fa4551509b1cfe5143` |
| `data/pack/script.cpz` | `CPZ5` | `bd70dba1785ec45494e73c451e6d492a91fd2eb0e56787dc60bce96c9b06effe` |

## 当前进展（2026-11-03）

- 已绑定两架构与游戏数据；各有864项分派（614个处理命令、250个空操作），处理命令保持 `Unreviewed`。游戏ID `0x17500003` 来自启动脚本 `0x201d` 的常量实参。
- 读取 `WINDOW_RESOLVING/BASE=1024,768`，统一场景、文字、命中、光标、影片与截屏尺寸；桌面和Android共用该画布，返回标题改用 `main.ps2`。
- 补齐窗口位置查询、存档槽元数据更新、封包模式2/3的场景影片创建及当前帧查询，减少交互页面与影片控制的未处理命令；证据及边界见[VM布局](vm-semantics.md#交互与影片命令2026-11-03)。
- 静态核对后开放文字v2剧情快照子集；拒绝按钮列表、≥512的嵌套子槽和未支持的活动效果。真实probe发现并修复 `0x2007` 整秒查询、`0x22a5` 类型5粒子/快照与回看中的整数常量赋值；证据及边界见[VM布局](vm-semantics.md#probe实测停点修复2026-11-03)。

## 真实probe验证（2026-11-03）

固定种子0、16ms帧、鼠标、日文字体，原游戏只读，用户状态位于各输出目录的隔离子目录。未复核命令保持研究放行，不提升语义证据等级。

| 输出（`local/miranosu/`下） | 已验证结果与限制 |
|---|---|
| `probe-baseline` | S1/S2/S3/S5、R1、D1通过；114个脚本、367种命令；剧情入口停在 `intproc.ps2 / 0x540a2 / 0x2007` |
| `probe-uptime` | R1/D1和剧情开头V1–V4通过；R2推进至 `snky01.ps2 / 0x29036 / 0x22a5`，4,155,596步/10,198帧 |
| `probe-pages` | P3/C1/C2通过，设置 `active_draw` 改动重启后保持 |
| `probe-pages-400` | P1发现51个状态、400次尝试、重放分歧0；预算耗尽，仍为inconclusive |
| `probe-input-saves` | D2/D3、I2/I3通过；I1/I4发现回看字面量赋值错误，已修复；V5因一千万步预算耗尽失败，不能视为系统存档往返通过 |
| `probe-backlog-fixed` | 修复后的I1/L0、I4/L1通过，回看打开后返回同一等待 |
| `particle-roundtrip-follow.log` | 真实活动类型5的256粒子保存/加载/再保存通过，60字节效果记录恢复成功，103,635字节快照逐字节一致；后续20个消息等待无额外前导即对齐 |
| `probe-final-first-route` | 最新构建R1/D1、V1–V4和R3通过；10次均选按钮0，261,846,063步/120,561帧回到标题，按钮从4个增至5个；R2因还有10个选择前缀未探索而inconclusive |

标题稳定页位于 `menu.ps2`、4个按钮，启动18,744步/624帧。最新首路线summary SHA-256 `70417c38882b6d473b0774eecc2452392fb6072ec8fc9ca4a5878bd8349632eb`。`probe-particle-snapshots` 的保存分支模式扩展探索已结束：16次均回到标题，无运行时停点；R3通过，R2因预算耗尽、42个选择前缀未探索而inconclusive。此轮不累计各路线通关系统数据，不能据此认领五结局或杏奈解锁通过。原生窗口交互、像素/听感和原版存档互操作未验证；上述存读档结果仅覆盖已记录站点。

## 启动

```bash
cargo run --offline --release --features desktop,audio-output --bin cmvs-desktop -- "<游戏目录>" --encoding shift-jis --audio native --font "<日文字体>" --user-data-dir "<独立用户目录>"
```

若停在尚未支持的活动效果快照，可显式加 `--continue-without-snapshot` 继续研究，该模式不能保存剧情快照。未复核命令默认研究放行，不表示语义已确认；可用 `--block-unreviewed` 保持严格闸门。

真实素材、IDA与本地分析输出留在忽略目录 `local/miranosu/`，原游戏保持只读。

本轮代码验证：core单元、计时器/共享分派、profile/粒子、整数/浮点/脚本数据及runner/快照相关回归通过；受影响目标严格Clippy、格式和差异检查通过。runner初次有3项临时目录权限失败，改用仓库内隔离临时目录后通过。本轮未重跑全workspace、Android或原生窗口验收；前期全功能1122项通过/25项忽略及Android检查是静态补全阶段记录。
