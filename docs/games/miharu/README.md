# 《みはる》测试档案

《あるみ箱（あると＋みはる）》收录的两个独立游戏之一，也是第一个接入的 CVNS（CMVS 前代引擎）作品。CVNS 接入机制见[引擎档案层第 16 节](../../design/engine-profiles.md#16-cvns-家族与命令翻译2026-11-14)，命令映射与复核记录见[命令映射](commands.md)。

## 样本身份

| 字段 | 内容 |
|---|---|
| 游戏标识 | `miharu` |
| 作品与本地样本目录 | 《あるみ箱（あると＋みはる）》中的《みはる》；`CVNS/2009-05-29_あるみ箱（あると＋みはる）/みはる` |
| 发行版本与补丁 | 未确认；目录日期仅作为样本标签，游戏目录带 `update.log` |
| 引擎 | CVNS；`cvns.cfg` 写有 `GAME_ID=1006`、`WINDOW_WIDTH=800`、`WINDOW_HEIGHT=600` |
| 样本平台与架构 | Windows x86 |
| 引擎版本标识 | 档案 `cvns-20090203`（按 PE 时间戳命名），与《あると》共用同一个 `cvns.exe` |
| 文本编码 | 未确认，推定 Shift-JIS |
| 可执行文件 | `cvns.exe` SHA-256 `8e6044466b928d77f0a94a0d0f32fe48190596bf628115598881c2aa3a4885aa`，PE 时间戳 0x49873a5b，链接器 9.0，image base 0x400000；switch 在 RVA 0x49c20，表 0x4c6bc，608 项 |
| 启动脚本 | `data/pack/start.pss`（PSS0）SHA-256 `834dc88750f547d341ee732a7116472d4fef18dd57a7bbe0ac9ba77dd2a87a09` |
| 脚本包 | `data/pack/script.cpz`（CPZ2，33 个脚本）SHA-256 `8c2c749799a4592e6a185a098d6da16d3c347b568874a5695eaeff53f1019f07` |
| 资源格式 | 图像 PB2A（`.pb2`）、遮罩 MSK0（`.msk`）、语音 MV2X（`.mv2`，由 `mog2.dll` 解码，内含 Vorbis 字样）、音效与 BGM 为 OGG；无视频 |

## 当前验证

| 项目 | 结果与范围 |
|---|---|
| 容器与脚本结构 | 全部 CPZ2 封包按条目校验和读取通过；34 个脚本（含 `start.pss`）线性解码通过 |
| 档案识别 | `cmvs-profile identify` 按数据与 exe 选中 `cvns-20090203` / `miharu` |
| 启动、标题与 OP | `start.pss` 用到的 29 种系统命令（共 36 次调用）已复核映射并执行，其中 17 种映射到 CMVS 根语义、12 种为 CVNS 专有，包括读取样本 `save/save000.dat`（CSS0）；随后进入 `intproc.pss`，停在 `0x46cb2 / 0x2029` |
| 剧情、选择与结局 | 未测试 |
| 画面与字体 | 未测试；PB2A、MSK0 解码器未实现 |
| 原生窗口、输入与音轨 | 未测试；MV2X 未实现 |
| 保存、关闭、重启与恢复 | 未测试 |
| 其他平台 | 未测试 |

## 重现与限制

- 测试日期：2026-11-14，分支 `feat/cvns`。
- 运行命令：`cmvs-trace <みはる>/data/pack/start.pss --script-archive <みはる>/data/pack/script.cpz --record-platform <みはる> --bootstrap --save-dir <空目录> --system-save <みはる>/save/save000.dat --continue-without-snapshot --steps 100000`。
- 当前停点：`intproc.pss / 0x46cb2 / 0x2029`。
- 运行时差异：字符串变量区按档案设为 256 槽（CVNS 的 128–255 是持久区）；变量区初值按 `.data` 零填充登记（`EngineProfile::initial_globals`）。存档菜单扫描的文件号是槽号 +1。
- 游戏 ID：记录为 0，尚未确认 CVNS 是否有命令把游戏 ID 交给引擎。
