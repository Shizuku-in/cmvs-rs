# Switch Homebrew（开发中）

实验性 NRO 播放器：deko3d GPU 合成（可切回软件渲染）、文字、Vorbis 音频、手柄与触摸、存档、软键盘及多游戏选择。未知游戏与未实现指令会报错。

## 构建

在仓库根目录执行（需要 Docker）：

```sh
docker build -t cmvs-switch-build platforms/switch
docker run --rm -v "$PWD:/src" -w /src cmvs-switch-build sh platforms/switch/scripts/build-player.sh
```

产物为 `platforms/switch/build/cmvs-switch.nro`。`scripts/build-smoke.sh` 另生成诊断程序 `cmvs-switch-smoke.nro`（检查文件读取、核心 SHA-256，按 A 检查游戏初始化），保持原名放在同一目录。

## 部署

SD 卡目录 `switch/cmvs-rs/`：

```text
cmvs-switch.nro
games/<任意名称>/   每个目录放一个原始游戏
game/               旧的单游戏目录，仍可用，列在最前
font.ttf            可选，字体索引 0；缺省用系统共享字体
renderer.txt        可选，gpu（缺省）或 cpu
```

程序生成：`users/<游戏标识>/`（用户数据，同一游戏的多个目录共用）、`last-game.txt`（上次启动的游戏）、`error.log`（最近一次错误）。

通过 hbmenu 的应用接管模式启动。只有一个游戏时直接启动；多个时显示列表，名称即目录名，选中后才识别游戏。存档时间与桌面、Android 一致按 UTC 记录。

## 操作

**游戏列表**

| 输入 | 作用 |
|---|---|
| 方向键 / 摇杆上下 | 移动，按住 400 ms 后连续移动 |
| L / R | 翻页 |
| A，或点击已选中的行 | 启动 |
| + | 退出程序 |

**游戏内**：游戏动作由 `key.cfg` 决定，宿主只把按键映射到引擎输入码。

| 输入 | 引擎输入 |
|---|---|
| 左摇杆 | 移动指针（死区外二次曲线，满偏约 1.2 秒横穿画面） |
| 触摸 | 画面内按下即左键并移动指针；黑边起手、第二指不生效 |
| A / B | 鼠标左 / 右键 |
| 方向键 | 方向键（引擎按钮导航，可移动指针） |
| Y / X | Enter / Esc |
| ZR / ZL | Ctrl / Shift |
| L、R、右摇杆上下 | 滚轮上 / 下，按住 400 ms 后每 100 ms 重复 |
| 长按 + 1 秒 | 保存设置并离开游戏（有列表时回到列表） |

- 游戏自身退出时同样回到列表；从 HOME 关闭则先保存设置再退出程序（相册小程序模式下系统可能直接终止）。
- 切到 HOME 或覆盖层时输入取消、音频静音，引擎按游戏的后台绘制设置暂停。
- 进入列表、游戏或关闭系统小程序时仍按住的按键，松开前不生效。

## 错误

错误写入 `error.log`（含脚本名、PC 与步数）并显示错误页：有列表时 A 回到列表、+ 退出，否则 A / + 退出。无法绘制错误页时（窗口打开前、GPU 失败、字体不可用）改用系统错误对话框，完整内容在“详情”中。GPU 失败不会自动换回 CPU。Rust panic 直接中止，消息仍写入 `error.log`。

## 实现

| 部分 | 说明 |
|---|---|
| GPU 画面 | `runtime/gpu.c` 用 deko3d 执行与 cmvs-wgpu 相同的绘制队列（参数布局见 `cmvs_core::scene_render::shader`）；`runtime/shaders/*.glsl` 移植自 wgpu 着色器，构建时由 `uam` 编译嵌入。逻辑画布按显示设置缩放到 1280×720 交换链（双缓冲、垂直同步）。纹理按内容缓存，上限 128 MiB、LRU 淘汰；影片帧不进缓存。每帧开始时回收上一帧内存，GPU 与下一段 VM 执行重叠；截图只在引擎请求时读回 |
| CPU 画面 | 核心软件合成后写入 1280×720 线性帧缓冲 |
| 加载提频 | 启动及 VM 连续 50 ms 未产出画面时请求 FastLoad（CPU 1785 MHz，系统同时把 GPU 降到最低），画面正常 500 ms 后恢复。只计 VM 时间，失焦时不提频，系统拒绝后不再尝试 |
| 音频 | `cmvs-audio` 的 `mixer` 模式，专用线程在 CPU 核心 1 以 48 kHz 立体声写入 audout，最多约 85 ms 排队 |
| 文本输入 | 系统软键盘，上限沿用桌面宿主（最多 127 个全角字）；按 Shift-JIS 保存，无法表示的字符在键盘内要求修改；与原版一致只保留全角字符 |
| 菜单与错误页 | CPU 绘制（`ab_glyph`），经当前渲染器显示 |
| 运行时补丁 | `runtime/libc.c` 补上 Rust std 所需而 newlib 缺少的 `sysconf` / `getrandom` |

输入、列表、错误页、提频与软键盘编码的逻辑可在开发机上 `cargo test -p cmvs-switch`。`menu_preview` / `error_preview` 两个忽略测试可把页面渲染为 PNG（需 `CMVS_MENU_FONT` 指定字体）。

暂未接入字体选择对话框。

## 测试状态

2026-11-06，Ryujinx 1.3.3（固件 22.5.0，主机模式）：

| 项目 | 结果 |
|---|---|
| 《アマツツミ》CPU 渲染 | 启动、系统字体文字、BGM、手柄、存档（同槽覆盖、移动、特殊槽）与长按 + 保存设置通过 |
| 同作 GPU 渲染 | 画面与文字正确，60 FPS 流畅，存档正常 |
| 游戏列表 | 四个条目显示、启动无误触、回列表后换游戏、记住上次选择通过 |
| 错误显示 | 错误页与系统错误对话框（应用版）通过 |

未验证：真机（含提频效果，Ryujinx 不模拟频率）、触摸（需掌机模式）、软键盘（该作无存档注释）、HOME 关闭保存（Ryujinx 退出锁为空实现）、GPU 与 CPU 的逐像素对比，以及影片、遮罩过渡与世界公告板的 GPU 路径。
