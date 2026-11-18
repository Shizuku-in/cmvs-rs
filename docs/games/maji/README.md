# 《まじぷり -Wonder Cradle-》测试档案

《まじぷり -Wonder Cradle-》（`maji`）使用早期 2004 年 CVNS 引擎构建（`cvns-20040515`）；引擎映射规则见[CVNS 映射](../../design/engine-profiles.md#cvns)。

## 样本身份

| 字段 | 内容 |
|---|---|
| 游戏标识 | `maji`（《まじぷり -Wonder Cradle-》） |
| 发行版本与补丁 | 初版目录 `2004-06-04_まじぷり -Wonder Cradle-`（原始文件日期 2004-04-07–2004-05-19；另有运行生成的 `cvns.cfg` 与 `data/save/save000.dat`） |
| 引擎与版本 | CVNS / 档案 `cvns-20040515`（Windows x86；`cvns.cfg`：`GAME_ID=1001`、`WINDOW_SIZE=0`、`640×480`，`SAVE_COUNT=90`，Shift-JIS） |
| 可执行文件 | `cvns.exe`（image base `0x400000`，时间戳 `0x40a51966` / 2004-05-15 JST，链接器 `7.10`，switch RVA `0x32555`、table `0x3300c`、default `0x32587` / 544 项）SHA-256 `8d5abea307067f2789ae3a7fb4a9cd09aa4e48393a06e3d172ca4250dc56274f` |
| 启动与脚本包 | `data/pack/start.pss`（PSS0）SHA-256 `819bdf49cec478d6ff04939c686f77fba7673fd80eb6d8575a60de9dff62fc55`；`data/pack/script.cpz`（CPZ1，89 项）SHA-256 `e55d699f671d47479677b406d0c5d947ab9d22e369c93af8653665af284775de` |
| 资源与存档格式 | `data/pack/` 下 10 个 `CPZ1` 封包（`bg`/`chip`/`data`/`event`/`script`/`se`/`stand`/`voice`/`voice2`/`voice3`）、`data/music/` 下 33 个独立 `.ogg`、`data/video/op.cmv`（`CMV6` 640×480 30fps 3,359 帧）、`data/icon/` 下 32×32 光标 `01.cur`–`07.cur`（6 枚 8-bit 及 `04.cur` 4-bit `BI_RGB`）；`data/save/save000.dat` 为 `CSS0`（2,679B），剧情存档为 `CSP1` |

## 当前验证

| 项目 | 结果与范围 |
|---|---|
| 封包与脚本 (`S1`–`S4`) | 通过（`--render-frames`）：9 个资源封包及音乐/视频共 19,846 项资源（`cmv` 1、`msk` 43、`mv2` 13,436、`ogg` 276、`pb2` 6,090）与全部 89 个脚本（7,210,901 条指令、209 条已去重系统命令全部为 `reviewed`；[cvns-20040515.profile](../../../crates/cmvs-profile-research/profiles/cvns-20040515.profile) 共 246 `reviewed` / 69 `unreviewed` / 229 `noop`）通过；`op.cmv` 3,359 帧离线解码通过（3,968 ms）；`04.cur` 4-bit `BI_RGB` 导出 4,096 字节 RGBA 与 Pillow 逐字节一致 |
| 系统存档 (`S5`) | 跳过：游戏根目录无 `save/` 目录 |
| 启动与确定性 (`R1`, `D1`) | 通过（含 `--render-frames`）：无 `save000.dat` 冷启动经 `logo.pss` → `pro.pss` → `1kyo00.pss` 首个 3 按钮选择页（`0x78e70`，877,564 步 / 16,034 帧 / 89 次推进，状态哈希 `d204e3d7c07aff1d7a9f4917d9f2c7612456837cdd2c97fb3e240825ba68dfa1`），重放 16,034 帧轨迹与末状态哈希一致 |
| 交互与回想 (`I1`, `I4`) | 通过（含 `--render-frames`）：在 `1kyo00.pss` 等待点通过全部输入动作，滚轮上滑打开 99 条回想记录并右键返回 `1kyo00.pss` `0x79fac` 原等待点 |
| 存读档恢复 (`V1`–`V4`) | 通过（含 `--render-frames`）：`CSP1` 重启读档（带 `save000.dat` 经 `menu.pss` 读回 `1kyo00.pss`）、再保存字节一致、同会话读档及 `998/999` 备份槽均通过（`1kyo00.pss` 21 步等待序列一致） |
| 系统存档与损坏档 (`D3`, `V5`) | `D3` 通过。`V5` 探针在观察到路线结束时未收到系统存档 |

## 命令映射与复核

- **基础环境与复用**：经 `cvns.exe`（`8d5abea3...`）核对，`sub_401250` 按 `CVNS_SYSTEM_MAIN.WINDOW_SIZE` 选 `0→640×480`、`1→800×600`、`2→1024×768`（本样本为 `0`）；`sub_411670` / `sub_412170` 读写 `CSS0` 与 `[4096, 262144, 0, 256]` 全局变量；`sub_4312E0` 读写 `CSP1`；`sub_424850` `case 0x413` 与 `sub_423CE0` 分别实现 `0x0413` 按 `LOWORD(4 * result)` 退栈与空 `0x0200 0x020f` 返回 `0`；`0x20e8`（`sub_42B3C0`）、`0x2143`（`sub_42D070`）、`0x21b7`（`sub_42EA20`）、`0x212c`（`sub_42EDF0` → `sub_4216E0`）、`0x2136`（`sub_42EFD0` → `sub_421440`）、`0x21ab`（`sub_42FE80`）、`0x2200`（`sub_430DA0`）、`0x2201`（`sub_430DF0`）与 `cvns-20040730` 一致；与 `cvns-20040730` 相比仅 `0x2064`/`0x2065` 在本构建为 `noop`（`0x32fd5`），其余 246 条已复核命令沿用 [cvns-20040730](../../../crates/cmvs-profile-research/profiles/cvns-20040730.profile) 映射，69 个未使用槽位保持 `unreviewed`。

## 重现与限制

- **验证命令（2026-11-17，分支 `feat/cvns`）**：
  - 探针（含渲染）：`cargo run --offline --release -p cmvs-probe -- "<まじぷり根目录>" --out /tmp/maji_probe_final --font /usr/share/fonts/opentype/ipafont-mincho/ipamp.ttf --render-frames --checks S1,S2,S3,S4,S5,R1,D1,I1,I4,V1,V2,V3,V4 --max-steps 5000000`
  - 视频：`cargo run --offline --release --bin cmvs-cmv -- "<まじぷり根目录>/data/video/op.cmv" --check-frames`
  - 桌面运行参考：`cargo run --offline --release --features wgpu-renderer,audio-output --bin cmvs-desktop -- "<まじぷり根目录>" --encoding shift-jis --renderer wgpu --audio native --font /usr/share/fonts/opentype/ipafont-mincho/ipamp.ttf --user-data-dir <用户目录>`
- **限制**：长路线、系统存档持久化及桌面实机交互仍待验证。
