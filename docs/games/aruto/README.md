# 《あると》测试档案

《あるみ箱（あると＋みはる）》收录的两个独立游戏之一，与[《みはる》](../miharu/README.md)共用同一份 `cvns-20090203` 可执行文件；引擎映射规则见[CVNS 映射](../../design/engine-profiles.md#cvns)。

## 样本身份

| 字段 | 内容 |
|---|---|
| 游戏标识 | `aruto`（《あるみ箱（あると＋みはる）》中的《あると》） |
| 发行版本与补丁 | `update.log` 记 `UPDATE_VER=100`、`UPDATE_DAY=2009/05/29` |
| 引擎与版本 | CVNS / 档案 `cvns-20090203`（Windows x86；`cvns.cfg`：`GAME_ID=1005`、`800×600`，Shift-JIS） |
| 可执行文件 | `cvns.exe`（image base `0x400000`，时间戳 `0x49873a5b`，switch RVA `0x49c20` / 608 项）SHA-256 `8e6044466b928d77f0a94a0d0f32fe48190596bf628115598881c2aa3a4885aa` |
| 启动与脚本包 | `data/pack/start.pss`（PSS0）SHA-256 `8733206a83c7d249ab9245d85d9dfe0b7125c3d2d85e0b2f730000d79dd35bc9`；`data/pack/script.cpz`（CPZ2，124 个脚本）SHA-256 `b62efd2250283532e8ac3a01d1ce46846b28900811a26f58d8296faf8d22d5f7` |
| 资源格式 | `PB2A`、`.msk`（`MSK0`）、`.mv2`（`MV2X`）、OGG（BGM / SE）、`data/video/op.cmv`（`CMV3` 800×600 24fps） |

## 当前验证

| 项目 | 结果与范围 |
|---|---|
| 封包与脚本 (`S1`–`S4`) | 通过：7 个 CPZ2 / 24,857 项资源头及 124 个脚本（8,429,520 条指令、全部 243 条系统命令在 `cvns-20090203` 为 `reviewed`，含 `0x20c0 → map=0x24c0`）通过；`op.cmv` 2,174 帧离线解码通过（3780 ms） |
| 系统存档 (`S5`) | 跳过：样本 `save/` 目录无自带存档 |
| 启动与确定性 (`R1`, `D1`) | 通过（含 `--render-frames`）：冷启动经 `logo.pss` 进入 `menu.pss` 4 按钮起始页（27,016 步 / 624 帧），重放哈希一致 |
| 桌面实机 (`wgpu` + 音频) | 已验证冷启动 → 标题 → 序章 → OP 播放（`op.cmv` 过 `20044 ms`，左键跳过、`Ctrl` 不误断）→ 进入 `６月１日【月曜日】` 显示对话并播放 BGM/SE/MV2 语音，经游戏内退出保存并正常退出（`1,061,830` 步 / `109,019` 次已复核命令 / 0 报错） |
| 自动交互与存读档 | `D3`、`I1`/`I3`/`I4`、`V1`–`V5` 通过。重启及同会话读档均与原进度对照 21 个等待点 |
| 自动路线探索 (`R2`, `R3`) | 8 条路径中 5 条回标题，3 条合并到已见路径。仍有 110 个前缀待探索 |

## 重现与限制

- **验证命令（2026-11-17，分支 `feat/cvns`）**：
  - 探针与视频：`cargo run --offline --release -p cmvs-probe -- <あると目录> --out /tmp/aruto_probe --font .../ipamp.ttf --checks R1,D1 --render-frames`、`cargo run --offline --release --bin cmvs-cmv -- <あると目录>/data/video/op.cmv --check-frames`
  - 桌面实机：`cmvs-desktop <あると目录> --encoding shift-jis --renderer wgpu --audio native --font .../ipamp.ttf --user-data-dir <用户目录>`
- **限制**：桌面验证覆盖到 OP 后的开篇。全分支、设置和鉴赏仍待验证。
