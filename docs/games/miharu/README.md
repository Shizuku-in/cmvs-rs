# 《みはる》测试档案

《あるみ箱（あると＋みはる）》收录的两个独立游戏之一，也是首个接入的 CVNS（CMVS 前代引擎）作品；引擎接入规则见[CVNS 映射](../../design/engine-profiles.md#cvns)。

## 样本身份

| 字段 | 内容 |
|---|---|
| 游戏标识 | `miharu`（《あるみ箱（あると＋みはる）》中的《みはる》） |
| 引擎与版本 | CVNS / 档案 `cvns-20090203`（Windows x86；`cvns.cfg`：`GAME_ID=1006`、`800×600`；与《あると》共用 exe，文本编码 Shift-JIS） |
| 可执行文件 | `cvns.exe`（image base `0x400000`，时间戳 `0x49873a5b`，switch RVA `0x49c20` / 608 项）SHA-256 `8e6044466b928d77f0a94a0d0f32fe48190596bf628115598881c2aa3a4885aa` |
| 启动与脚本包 | `data/pack/start.pss`（PSS0）SHA-256 `834dc88750f547d341ee732a7116472d4fef18dd57a7bbe0ac9ba77dd2a87a09`；`data/pack/script.cpz`（CPZ2，33 个脚本）SHA-256 `8c2c749799a4592e6a185a098d6da16d3c347b568874a5695eaeff53f1019f07` |
| 资源格式 | `PB2A`（`.pb2`）、`MSK0`（`.msk`）、`MV2X`（`.mv2`，由 `mog2.dll` 解码 Vorbis 语音）、OGG（BGM / SE）；无视频 |

## 当前验证

| 项目 | 结果与范围 |
|---|---|
| 封包与脚本 (`S1`–`S4`) | 通过：6 个 CPZ2 / 5,283 个资源头（`msk` 43、`mv2` 4161、`ogg` 319、`pb2` 760）解码；33 个打包脚本（1,511,961 指令）全部解码，使用的 242 条系统命令在档案中均为 `reviewed` |
| 系统存档 (`S5`, `D3`) | 通过：随盘 `save000.dat`（CSS0）解码通过；损坏/截断系统存档及缺失/损坏剧情存档均干净拒绝 |
| 启动与确定性 (`R1`, `D1`, `D2`) | 通过：冷启动经 `logo.pss` 进入 `menu.pss` 5 按钮起始页（562 帧），重放摘要一致，`16 ms`/`33 ms` 通过 `smop.pss / 0x2bd34` 相同 40 个等待点 |
| 画面渲染与输入 (`I1`–`I5`) | `--render-frames` 下 `I1`/`I3`/`I5` 通过（3000 帧随机输入无报错）；`I2` 因档案无 `key-config` 跳过；`I4` 在开场第 10 个等待点录得 `inconclusive`（`wheel up showed nothing new`） |
| 剧情存档恢复 (`V1`–`V4`) | cmvs-rs 写出的 `CSP1` 通过同会话与重启恢复、21 步等待序列对照、字节往返及备份槽检查 |
| 原版剧情存档 | 3 个原版槽无窗口读档后均继续到下一个快照点，续玩后写档可再读。另检查了历史条目、BGM 状态和场景渲染。 [兼容范围](../../formats/save-format.md#cvns-csp1) |
| 桌面实机 (`wgpu` + 音频) | 已实机运行并通过美春线（标题 → 序章 → 7/8 月日常 → `スタッフロール【美春】` → `エピローグ【美春】` → 正常退出，`31,862,956` 步 / `3,525,026` 次已复核命令 / 0 报错，`wgpu` Vulkan 渲染、BGM/SE/MV2 语音与 `save000`/`001`/`860` 写出正常） |
| 未完成与未覆盖范围 | `V5`、`P1`–`P3`、`C1`–`C4` 长路径自动探测未运行完；其余支线及 Linux 以外平台未测试 |

## 重现与限制

- 探针使用 `--checks S1,S2,S3,S4,S5,R1,D1,D2,D3,I1,I2,I3,I4,I5,V1,V2,V3,V4 --render-frames`。 [探针入口](../../guides/cli.md#通用探针)
- 桌面使用 Shift-JIS、wgpu、原生音频和日文字体。原版无 `key.cfg`，桌面使用默认键鼠映射。`0x20ca` 逐帧提供点击矩形。 [运行指南](../../guides/running.md)

原版读档回归使用上述《みはる》样本及验证用的 `save/save000.dat` 至 `save/save003.dat`。

```sh
CMVS_CVNS_GAME_ROOT="<样本目录>" cargo test --offline --release -p cmvs-desktop --test save_slots external_cvns_original_slots_resume -- --ignored
```

## 命令映射

档案绑定本页 x86 样本，使用根语义与 `0x24xx`–`0x26xx` CVNS 专用处理器。 [cvns-20090203.profile](../../../crates/cmvs-profile-research/profiles/cvns-20090203.profile)

- 整数 4096、位 262144、字符串 256 槽。持久起点由 `0x210b`–`0x210d` 设定。`save000.dat` 保留给 CSS0，`0x21a1` 从菜单索引 N 查找 `save(N+1).dat`。 [剧情存档格式](../../formats/save-format.md#cvns-csp1)
- **场景、消息与表达式**：128 槽 16 层图像缓冲区（`0x205e`/`0x205f`/`0x2061`/`0x2063`/`0x2067`/`0x2218` 对有效槽内的空子图层及 `layer >= 16` 遍历遇首个空图层均静默无操作）、1536 精灵、8 文本窗与 4 消息框（4096 字形图集、`0x446660` 自动回调 `10..=13` 按 `messages[0..3].visible` 触发）；`0x2137` 将 `arg1` 写入动态正文 `strings[2]`、`arg2` 写入 32 字节字段 `strings[1]`；表达式支持 `0x107`/`0x10a`/`0x10b`/`0x111`–`0x113` 一维/二维步长数组与帧标量。
- **音频与兼容放行**：支持 BGM（含 `loop_sample`）、48 路 SE、4 路 MV2 语音与 4 路辅助音效；`0x2017`（模式 `2/3/5/6`）与 `0x202e`（模式 `1/2/3/5/6`）按 `activation-pass` 返回通过，不访问原版驱动器、进程或注册表。
