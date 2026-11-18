# 《秋色恋華》测试档案

《秋色恋華》（`aki`）使用 2005 年初版 CVNS 引擎构建（`cvns-20050216`）；引擎映射规则见[CVNS 映射](../../design/engine-profiles.md#cvns)。

## 样本身份

| 字段 | 内容 |
|---|---|
| 游戏标识 | `aki`（《秋色恋華》） |
| 发行版本与补丁 | 初版（目录文件日期 2005-02-16–2005-02-18，游戏标题目录 `2005-02-25_秋色恋華`） |
| 引擎与版本 | CVNS / 档案 `cvns-20050216`（Windows x86；`cvns.cfg`：`GAME_ID=1003`、`WINDOW_SIZE=1`、`800×600`，`SAVE_COUNT=90`，Shift-JIS） |
| 可执行文件 | `cvns.exe`（image base `0x400000`，时间戳 `0x42121eb1` / 2005-02-16 JST，链接器 `7.10`，switch RVA `0x35a05` / 560 项）SHA-256 `1e0d17db759650c32e94a9bc1eb6a143463df22638fba687b420644dce969e42` |
| 启动与脚本包 | `data/pack/start.pss`（PSS0）SHA-256 `3c2507628fecfabd57a17d9f306fb9faf9dc42115fc10fb88bba35b52a5c03a9`；`data/pack/script.cpz`（CPZ1，111 项，含同名前缀 `/start.pss`）SHA-256 `a1a120a72fac0caa77b5cd93338da58b12b6b3e13681211aa0603f0ee388c050` |
| 资源与存档格式 | 9 个 `CPZ1` 封包、`PB2`（`.pb2`）、`MSK0`（`.msk`）、`MV2X`（`.mv2`）、OGG（含大写 `BGM*.ogg`）、`op.cmv`（`CMV6` 800×600 24fps 2,965 帧）、24-bit `BI_RGB` 光标 `01.cur`–`07.cur`；`CSS0`（`save000.dat`）与 `CSP1`–`CSP9` |

## 当前验证

| 项目 | 结果与范围 |
|---|---|
| 封包与脚本 (`S1`–`S4`) | 通过：8 个 CPZ1 资源档及音乐/视频共 13,818 项资源头（`pb2` 1,453、`msk` 41、`mv2` 12,124、`ogg` 199、`cmv` 1）与 111 个脚本（`data/pack/start.pss` 加 `script.cpz` 其余 110 个脚本，5,229,171 条指令、全部 208 条系统命令均为 `reviewed`；73 个未使用槽位保持 `unreviewed`，232 个空槽位为 `noop`）通过；`op.cmv` 2,965 帧离线解码通过（4,952 ms） |
| 系统存档 (`S5`) | 跳过：样本 `data/save/` 目录为空 |
| 启动与确定性 (`R1`, `D1`) | 通过（含 `--render-frames`）：冷启动经 `logo.pss` → `op.pss`（播完 `op.cmv`）进入 `menu.pss` 4 按钮起始页（14,502 步 / 8,377 帧），重放哈希一致 |
| 交互与回想 (`I1`, `I4`) | 通过（含 `--render-frames`）：在 `sx01.pss` 第 10 个等待点通过全部输入动作，滚轮上滑打开 10 条回想记录并取消返回原等待点 |
| 存读档恢复 (`V1`–`V4`) | 通过（含 `--render-frames`）：`CSP1` 重启读档、同会话读档、再保存字节一致及 `998/999` 备份槽均通过（`sx01.pss` 21 步等待序列一致） |
| 系统存档与损坏档 (`D3`, `V5`) | 通过：损坏档被拒绝，单线结束后重启恢复到同一标题页 |

## 命令映射与复核

- **基础环境与复用**：经 `cvns.exe`（`1e0d17db...`）核对，`sub_401270` 按 `CVNS_SYSTEM_MAIN.WINDOW_SIZE` 选 `0→640×480`、`1→800×600`、`2→1024×768`（本样本为 `1`）；`sub_412980`/`sub_411F00` 读写 `CSS0`，全局容量 `[4096, 262144, 0, 256]`；`sub_428490`/`sub_439670` 读写 `CSP1`–`CSP9`；`sub_42C850`（`0x20c0 -> map=0x24c0`）按 `master * video / 100` 缩放音量播放 `op.cmv`。脚本使用的 205 条共有命令及未使用的 46 条非空槽位经 switch 与处理函数对比沿用 [cvns-20050727](../../../crates/cmvs-profile-research/profiles/cvns-20050727.profile) 映射；其余 73 个未使用槽位保持 `unreviewed`。

### 0x20ef

- **映射**：`0x20ef reviewed 0x3600b map=0x24ef`（`cvns.exe` SHA-256 `1e0d17db759650c32e94a9bc1eb6a143463df22638fba687b420644dce969e42`，x86，image base `0x400000`，switch case `0x43600b` → `sub_42D4F0`）。
- **语义**：弹出 `SP-4`（辅助音效通道 `0..4`）与 `SP-8`（百分比增益乘数），返回 `0x4008`。通道 `< 4` 时调用音频管理对象虚表 `+76`（通道号 `v2 + 60`，与 `0x20bd` 语音增益 `sub_42D790` 使用同一虚表槽），`>= 4` 置错误位 `0x8000`。

### 0x20ff

- **映射**：`0x20ff reviewed 0x36083 map=0x24ff`（同上 x86 身份，switch case `0x436083` → `sub_42D490`）。
- **语义**：弹出 `SP-4`（辅助音效通道 `0..4`），返回 `0x4004`。通道 `< 4` 时调用音频管理对象虚表 `+84`（通道号 `v1 + 60`，写出 `system_integer(4)` 与 `system_integer(5)`，与 `0x20bf` 语音立体声峰值 `sub_42D830` 使用同一虚表槽），`>= 4` 置错误位 `0x8000`；当前运行时与 `0x4bf` 一致写回 `(0, 0)`。

### 0x212c

- **映射**：`0x212c reviewed 0x361a4 map=0x252c`（同上 x86 身份，switch case `0x4361a4` → `sub_4320A0` → `sub_4239F0`）。
- **语义**：早期的 2 字版 CVNS 历史追加命令，弹出 5 个参数（返回 `0x4014`）：`SP-4`（语音串写入 32 字节字段 `+4` / `strings[0]`）、`SP-8`（正文写入动态串 `+68` / `strings[2]`）、`SP-12`（说话人写入 32 字节字段 `+36` / `strings[1]`）、`SP-16`/`SP-20`（两项整数写入 `+72`/`+76`，对应 `words[0..2]`）。对应查询 `0x2136`（`sub_432280` → `sub_423750`）对两项整数索引做 `a3 & 1`。

## 重现与限制

- **验证命令（2026-11-17，分支 `feat/cvns`）**：
  - 探针（含渲染）：`cargo run --offline --release -p cmvs-probe -- <秋色恋華目录> --out /tmp/aki_probe_render --font .../ipamp.ttf --render-frames --checks S1,S2,S3,S4,S5,R1,D1,I1,I4,V1,V2,V3,V4 --max-steps 5000000`
  - 视频：`cargo run --offline --release --bin cmvs-cmv -- <秋色恋華目录>/data/video/op.cmv --check-frames`
  - 桌面运行参考：`cargo run --offline --release --features wgpu-renderer,audio-output --bin cmvs-desktop -- <秋色恋華目录> --encoding shift-jis --renderer wgpu --audio native --font .../ipamp.ttf --user-data-dir <用户目录>`
- **限制**：长路线、全分支和桌面全篇交互仍待验证。
