# 《まじぷり -Wonder Cradle- ふぁんぢぃすく》（まじぷりＦＤ）测试档案

《まじぷり -Wonder Cradle- ふぁんぢぃすく》收录的 CVNS 作品根目录为光盘下的 `fd/`（`まじぷりＦＤ`，`cvns-20040730`；同盘另有旧引擎 `hb/hpb.exe` 不属于 CVNS）。引擎映射规则见[CVNS 映射](../../design/engine-profiles.md#cvns)。

## 样本身份

| 字段 | 内容 |
|---|---|
| 游戏标识 | `majifd`（《まじぷり -Wonder Cradle- ふぁんぢぃすく》 / `fd/`） |
| 发行版本与补丁 | 光盘 `fd/` 目录（文件日期 2004-07-27–2004-08-02；另有 `update/maji103.exe` 未混用） |
| 引擎与版本 | CVNS / 档案 `cvns-20040730`（Windows x86；`cvns.cfg`：`GAME_ID=1001`、`WINDOW_SIZE=0`、`640×480`，`SAVE_COUNT=90`，Shift-JIS） |
| 可执行文件 | `cvns.exe`（image base `0x400000`，时间戳 `0x4109453f` / 2004-07-30 JST，链接器 `7.10`，switch RVA `0x34485`、table `0x34f4c`、default `0x344b7` / 544 项）SHA-256 `2bfcb306f094a3968af7bc88e37a091305c628b06ddfbf0fce14268d66bf3a07` |
| 启动与脚本包 | `data/pack/start.pss`（PSS0）SHA-256 `2f505e8ac49ccc8be7cfc79861ab1d2390050300a011fad07fe80b2f721b8c4f`；`data/pack/script.cpz`（CPZ1，23 项）SHA-256 `9ecae4b71c801414925d505ff62103865283f6925088028d729e3d134a22d46e` |
| 资源与存档格式 | `data/pack/` 下 7 个 `CPZ1` 封包（`bg`/`chip`/`event`/`script`/`se`/`stand`/`voice`）与 `data/music/` 下 36 个独立 `.ogg`；`data/icon/01.cur`–`07.cur`；`data/video/` 与 `data/save/` 为空；`CSS0`（`save000.dat`）与 `CSP1` |

## 当前验证

| 项目 | 结果与范围 |
|---|---|
| 封包与脚本 (`S1`–`S4`) | 通过（`--render-frames`）：6 个资源封包共 4,241 项资源头（`pb2` 973、`msk` 43、`mv2` 3,135、`ogg` 90）与全部脚本（`script.cpz` 23 个脚本共 1,843,638 条指令、206 条已去重系统命令全部为 `reviewed`，外加启动脚本 `start.pss` 线性反汇编 260 条指令通过；[cvns-20040730.profile](../../../crates/cmvs-profile-research/profiles/cvns-20040730.profile) 共 248 `reviewed` / 69 `unreviewed` / 227 `noop`） |
| 系统存档 (`S5`) | 跳过：样本 `data/save/` 目录为空 |
| 启动与确定性 (`R1`, `D1`) | 通过（含 `--render-frames`）：冷启动经 `start.pss` → `logo.pss` → `menu.pss` 5 按钮起始页（15,522 步 / 687 帧，状态哈希 `c83041f670a462dbe8921c8ae12261fbfd024f1a217bc8faf5bcbc13a3a889da`），重放 687 帧轨迹与末状态哈希一致 |
| 交互与回想 (`I1`, `I4`) | 通过（含 `--render-frames`）：进入 `fdnn7.pss` 后通过全部等待点输入动作，滚轮上滑打开 10 条回想记录并右键返回 `fdnn7.pss` `0x92e66` 原等待点 |
| `MSK0` 字形 | `fde01.pss`、`fdb01.pss` 的 22 像素爱心已在真实游戏帧中绘制，字形透明度与素材图块对照通过 |
| 存读档探针 (`D3`, `V1`–`V5`) | 通过（含 `--render-frames`）：短路线 `fdnn7.pss` 存档后重启及同会话恢复均对照 14 个等待点，重存字节、备份槽及系统存档检查通过 |
| 待验证 | 完整分支、设置和鉴赏、桌面全篇交互 |

## 命令映射与复核

- **基础环境与复用**：经 `cvns.exe`（`2bfcb306...`）核对，`sub_401250` 按 `CVNS_SYSTEM_MAIN.WINDOW_SIZE` 选 `0→640×480`、`1→800×600`、`2→1024×768`（本样本为 `0`）；`sub_4117E0` / `sub_435A20`–`sub_435A40` 读写 `CSS0` 与 `[4096, 262144, 0, 256]` 全局变量；`sub_427490` / `sub_427EC0` / `sub_433C10` 读写 600 字节 header `version = 3` 的 `CSP1`（原版体内存块为早期 `0x100..0x900`，本仓库使用自建 `SnapshotLayout::CvnsCsp1`）。另据 `sub_426380` `case 0x413` 与 `sub_425810` 修复 `0x0413` 4 字节解码/按 `LOWORD(4 * result)` 退栈返回（`intproc.pss`）以及顶层空 `0x0200 0x020f` 整数表达式按 `0` 求值（`fdnn1.pss`）。脚本其余 201 条共有命令及 42 个未使用槽位经 switch 与处理函数比对沿用 [cvns-20050216](../../../crates/cmvs-profile-research/profiles/cvns-20050216.profile)（其中 `0x2006`、`0x202a`、`0x2113` 处理逻辑核对等价或安全，69 个未用槽位保持 `unreviewed`）。

### 0x20e8

- **映射**：`0x20e8 reviewed 0x34a4a map=0x24e8`（`cvns.exe` SHA-256 `2bfcb306f094a3968af7bc88e37a091305c628b06ddfbf0fce14268d66bf3a07`，x86，image base `0x400000`，switch case `0x434a4a` → `sub_42D1C0` → `sub_43BAE0`）。
- **语义**：弹出 `(x1, y1, x2, y2)` 共 16 字节（返回 `0x4010`），以 `atan2(y2 - y1, x2 - x1)` 计算整数角度 `[0, 360)` 写入 `system_integer(4)`，距离写入 `system_integer(5)`（与 `0x20ed` 相同但为 1 度刻度而非 `1/100` 度）。

### 0x2143

- **映射**：`0x2143 reviewed 0x34c8d map=0x2543`（同上 x86 身份，switch case `0x434c8d` → `sub_42EE70` → `sub_41CB10`）。
- **语义**：弹出 `SP-4`（消息窗口槽）、`SP-8`（局部精灵槽）、`SP-12`（可见标志 `!= 0`），返回 `0x400c`，更新该局部精灵可见性并刷新缓存绘制对象。

### 0x21b7

- **映射**：`0x21b7 reviewed 0x34e95 map=0x25b7`（同上 x86 身份，switch case `0x434e95` → `sub_430820`）。
- **语义**：`SP-4`、`SP-8` 的 RGB 值依次放入 `ChooseColorA` 的前两格自定义色，初始选中黑色。回收 8 字节并返回 `0x4008`。确认写 `system_integer(0) = 1` 和 `system_integer(4) = 0xRRGGBB`，取消只把 `system_integer(0)` 清零。
- **构建复核**：三个 Windows x86 样本的参数、色序转换、结果槽和回收量一致，均映射到 `0x25b7`。`cvns-20050216` 为 `0x436446 → 0x431cd0`，`cvns-20050727` 为 `0x43ba0e → 0x436f20`，`cvns-20090203` 为 `0x465560`。 [秋色恋華样本](../aki/README.md#样本身份) · [秋色謳華样本](../akifd/README.md#样本身份) · [あるみ箱样本](../miharu/README.md#样本身份)

### 0x2200

- **映射**：`0x2200 reviewed 0x34f3c map=0x2600`（同上 x86 身份，switch case `0x434f3c` → `sub_432BC0` → `sub_407C90`）。
- **语义**：弹出 `SP-4`（`.msk` 资源名，如 `font20ex.msk`、`font22ex.msk`）与 `SP-8`（字号，如 `20`、`22`），返回 `0x4008`，读取并登记 8 位 `MSK0` 外挂字形表供 `0x2201` 切块。

### 0x2201

- **映射**：`0x2201 reviewed 0x34f44 map=0x2601`（同上 x86 身份，switch case `0x434f44` → `sub_432C10` → `sub_407D90`）。
- **语义**：弹出 `SP-4`（图块序号）与 `SP-8`（16 位 Shift-JIS 字符编码，如 `33175` / `0x8197`），返回 `0x4008`，从已登记的外挂字形表中按各表字号切出对应正方形 8 位透明通道字模并登记到字形缓存。
- **绘制**：`sub_423550` 经 `sub_407E90` 按字符编码和字号查找图块。`sub_407410` 直接使用图块透明度，启用阴影时向右下偏移一像素。

## 重现与限制

- 探针（含渲染）：`cargo run --offline --release -p cmvs-probe -- "<游戏目录>/fd" --out <报告目录> --font <日文字体> --render-frames --checks R1,D1,D3,I1,I4,V1,V2,V3,V4,V5`
- [桌面运行指南](../../guides/running.md)
