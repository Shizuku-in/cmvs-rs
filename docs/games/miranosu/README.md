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
- 静态核对后开放文字v2剧情快照子集；拒绝按钮列表、≥512的嵌套子槽和未支持的活动效果。证据见[VM布局](vm-semantics.md)。本轮仅做静态分析与合成回归，未运行真实游戏；标题、路线、原版存档互操作及视听效果均待实机验收。

## 启动

```bash
cargo run --offline --release --features desktop,audio-output --bin cmvs-desktop -- "<游戏目录>" --encoding shift-jis --audio native --font "<日文字体>" --user-data-dir "<独立用户目录>"
```

若停在尚未支持的活动效果快照，可显式加 `--continue-without-snapshot` 继续研究，该模式不能保存剧情快照。未复核命令默认研究放行，不表示语义已确认；可用 `--block-unreviewed` 保持严格闸门。

真实素材、IDA与本地分析输出留在忽略目录 `local/miranosu/`，原游戏保持只读。

代码验证：全workspace全功能测试1122项通过、25项忽略；全目标严格Clippy、格式和差异检查通过；Android x86_64目标检查通过。真实游戏验收留待后续。
