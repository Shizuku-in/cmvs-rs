# 《秋色謳華 －秋色恋華ファンディスク－》测试档案

《秋色謳華》（`akifd`）使用早期的 2005 年 CVNS 引擎构建（`cvns-20050727`）；引擎映射规则见[CVNS 映射](../../design/engine-profiles.md#cvns)。

## 样本身份

| 字段 | 内容 |
|---|---|
| 游戏标识 | `akifd`（《秋色謳華 －秋色恋華ファンディスク－》） |
| 发行版本与补丁 | 初版（2005-08-12，`readme.txt` 2005-07-31）；目录另附外置更新程序 `akifd101.exe` |
| 引擎与版本 | CVNS / 档案 `cvns-20050727`（Windows x86；`cvns.cfg`：`GAME_ID=1003`、`WINDOW_SIZE=9`、`1024×614`，Shift-JIS） |
| 可执行文件 | `cvns.exe`（image base `0x400000`，时间戳 `0x42e657db` / 2005-07-27 JST，链接器 `7.10`，switch RVA `0x3af65` / 608 项）SHA-256 `fd475dda79e4a559d934b214494d314539915afde7a910b3d35269dd9cd4a028` |
| 启动与脚本包 | `data/pack/start.pss`（PSS0）SHA-256 `bc23cdd6e62f9a09953169b4fb71dcc3b6b5d58c4bd7aa9ccd501128dba2483a`；`data/pack/script.cpz`（CPZ2，40 个脚本）SHA-256 `d380b8065fdf05440047adc74b36d59cab345486e2534d0e06fb195ecab8c699` |
| 资源与存档格式 | `PB2A`、`.msk`（`MSK0`）、`.mv2`（`MV2X`）、OGG（含大写 `BGM*.ogg`）、`opfd.cmv`（`CMV3` 1024×614 24fps）、24-bit `BI_RGB` 光标 `01.cur`–`07.cur`；`CSS0`（`data/save/save000.dat`）与 `CSP1`–`CSP9` |

## 当前验证

| 项目 | 结果与范围 |
|---|---|
| 封包与脚本 (`S1`–`S4`) | 通过：7 个 CPZ2 / 8,708 项资源头及 40 个脚本（2,603,824 条指令、238 条系统命令均为 `reviewed`；2009 版新增而 2005 版未实现的 18 个槽位保留 `noop`）全部通过；`opfd.cmv` 3,270 帧离线解码通过（6258 ms） |
| 系统存档 (`S5`) | 跳过：样本 `save/` 目录不存在（随盘 `data/save/save000.dat` 为 `CSS0`，600B 头 / 2,676B） |
| 启动与确定性 (`R1`, `D1`) | 通过：冷启动经 `logo.pss` → `op.pss` 进入 `menu.pss` 9 按钮页（11,301 步 / 9,172 帧），重放哈希一致 |
| 桌面实机 (`wgpu` + 音频) | 通过单线通关：经 OP、标题菜单、おまけ、光标切换与多章节直至 `スタッフロール（翼）` 后正常退出码 `0`（`16,257,745` 步 / `1,325,048` 次已审命令 / 0 报错） |
| 存读档 | 多级菜单进入 `zstv00.pss` 后，重启和同会话各恢复 21 个对话等待点。重存快照、备份槽与损坏档拒绝检查通过 |

## 命令映射与构筑差异

- **基础环境与复用**：经 `cvns.exe` 验证，全局容量 `[4096, 262144, 0, 256]`、`CSS0`/`CSP1`–`CSP9`、`WINDOW_SIZE=9 -> 1024×614` 及 `0x20c0`（`0xc004`）均与 `cvns-20090203` 一致；脚本使用的 232 条共有命令沿用 `cvns-20090203` 映射（其中 `0x21ab` 修正为 `map=0x25ab`），其余 77 个未使用槽位保持 `unreviewed`。
- **2005 版新增 6 条命令**：`0x20ec`（`0x3b58b`→`sub_4331F0`→`0x24ec`）与 `0x20ed`（`0x3b593`→`sub_433290`→`0x24ed`）分别为 `1/100` 度整角极坐标/直角坐标互转（写 `system_integer(1/2)` 与 `(4/5)`）；`0x2134`（`0x3b786`→`sub_434930`→`0x2534`）查询局部图像宽高，`0x2135`（`0x3b78e`→`sub_4348A0`→`0x2535`）做 240 刻度 HLS 染色，`0x213e`（`0x3b7d6`→`sub_434F60`→`0x253e`）置 0..256 不透明度，`0x2148`（`0x3b826`→`sub_434CE0`→`0x2548`）按原 `i16` 矩形绑定局部精灵。
- **实机兼容**：支持根路径内 ASCII 大小写无关目录项回退（`BGM01.ogg`–`BGM31.ogg`）、24-bit `BI_RGB` 单图光标解码（`01.cur`–`07.cur`），以及 `variant_text_dialog` 对 `intproc.pss`（`ae61c`/`b00f0`）负 `0x121` 槽位（`16 + (g[84] - 80)`）折叠高位 `StringRef::Frame` 初始串按原版 `lstrcpyA` SEH 行为回退为空串。

## 重现与限制

- **验证命令（2026-11-17，分支 `feat/cvns`）**：
  - 探针与视频：`cargo run --offline --release -p cmvs-probe -- <秋色謳華目录> --out /tmp/akifd_probe --font .../ipamp.ttf`、`cargo run --offline --release --bin cmvs-cmv -- <秋色謳華目录>/data/video/opfd.cmv --check-frames`
  - 桌面实机：`cmvs-desktop <秋色謳華目录> --encoding shift-jis --renderer wgpu --audio native --font .../ipamp.ttf --user-data-dir <用户目录>`
- **限制**：探针尚不能自动从多级标题菜单定位剧情入口。全分支、系统存档跨重启与 77 个未使用命令槽仍待验证。
