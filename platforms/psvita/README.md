# PS Vita Homebrew（开发中）

实验性 VPK 播放器：GXM GPU 合成（可切回软件渲染）、文字、Vorbis 音频、按键与前触摸屏、存档、系统输入法及多游戏选择。需要已破解（HENkaku / Ensō）的主机。未知游戏与未实现指令会报错。

## 构建

在仓库根目录执行（需要 Docker）：

```sh
docker build -t cmvs-vita-build platforms/psvita
docker run --rm -v "$PWD:/src" -w /src cmvs-vita-build sh platforms/psvita/scripts/build-player.sh
```

产物为 `platforms/psvita/build/cmvs-vita.vpk`（标题 ID `CMVS00001`）。GPU 着色器以编译好的 GXP 提交在 `shaders/`，构建不需要着色器编译器；修改着色器的流程见 [shaders/README.md](shaders/README.md)。`scripts/build-probe.sh` 另生成工具链诊断程序 `cmvs-vita-probe.vpk`（`CMVS00099`），检查文件、线程、64 MiB 堆与核心 SHA-256，结果写入 `ux0:data/cmvs-rs/probe.log`。

## 部署

用 VitaShell 安装 VPK，游戏放在 `ux0:data/cmvs-rs/`：

```text
games/<任意名称>/   每个目录放一个原始游戏
  encoding.txt      可选，游戏文字编码 shift-jis、gbk 或 utf-8；缺省从启动脚本自动检测
game/               旧的单游戏目录，仍可用，列在最前
font.ttf            可选，字体索引 0；缺省用系统字体 sa0:data/font/pvf/jpn0.pvf
renderer.txt        可选，gpu（缺省）或 cpu
```

游戏文字编码（Shift-JIS、GBK、UTF-8）默认从启动脚本自动检测，判断不了时会报错，在该游戏目录放 `encoding.txt` 写明编码即可。系统字体 jpn0.pvf 是日文字体，简体中文游戏需要放一个含简体中文字形的 `font.ttf`。

程序生成：`users/<游戏标识>/`（用户数据，同一游戏的多个目录共用）、`last-game.txt`（上次启动的游戏）、`error.log`（最近一次错误）。

只有一个游戏时直接启动；多个时显示列表，名称即目录名，选中后才识别游戏。存档时间按 UTC 记录。

## 操作

确定键跟随系统设置：日版及亚洲版为 ○，其他地区为 ×，另一个为取消。

**游戏列表**

| 输入 | 作用 |
|---|---|
| 方向键 / 摇杆上下 | 移动，按住 400 ms 后连续移动 |
| L / R | 翻页 |
| 确定键，或点击已选中的行 | 启动 |
| START | 退出程序 |

**游戏内**：游戏动作由 `key.cfg` 决定，宿主只把按键映射到引擎输入码。

| 输入 | 引擎输入 |
|---|---|
| 左摇杆 | 移动指针（死区外二次曲线，满偏约 1.2 秒横穿画面） |
| 前触摸屏 | 画面内按下即左键并移动指针；黑边起手、第二指不生效 |
| 确定 / 取消 | 鼠标左 / 右键 |
| 方向键 | 方向键（引擎按钮导航，可移动指针） |
| □ / △ | Enter / Esc |
| R | Ctrl（按住快进） |
| SELECT | Shift |
| L、右摇杆上下 | L 为滚轮上；右摇杆上下为滚轮上 / 下；按住 400 ms 后每 100 ms 重复 |
| 长按 START 1 秒 | 保存设置并离开游戏（有列表时回到列表） |

- 游戏自身退出时同样回到列表。
- 系统界面覆盖（快捷菜单等）时输入取消、音频静音，引擎按游戏的后台绘制设置暂停；从 LiveArea 恢复后同样处理，按住的按键松开前不生效。
- 系统不会通知应用关闭：在 LiveArea 直接关闭会丢失尚未保存的设置，请先长按 START 离开游戏。

## 错误

错误写入 `error.log`（含脚本名、PC 与步数）并显示错误页：有列表时确定键回到列表、START 退出，否则两者都退出。无法绘制错误页时（如字体不可用）改用系统消息框；显示初始化失败时只写日志。GPU 初始化失败同样报错，提示改用 `renderer.txt` 的 `cpu`，不会自动换回软件渲染。Rust panic 直接中止，消息仍写入 `error.log`。

## 实现

| 部分 | 说明 |
|---|---|
| GPU 画面 | `runtime/gpu.c` 在 vita2d 的 GXM 上下文中执行与 cmvs-wgpu 相同的绘制队列（参数布局见 `cmvs_core::scene_render::shader`，纹理缓存与 Switch 共用 `cmvs_console::gpu`）。逻辑画布是独立渲染目标，交给 `runtime/window.c` 按显示设置缩放到 960×544。纹理先放视频内存（8 MiB 分块），用尽后转主内存；缓存上限 64 MiB、LRU 淘汰。每帧开始时等待 GPU 并回收上一帧释放的纹理，GPU 与下一段 VM 执行重叠；截图只在引擎请求时读回。影片帧与宿主页面仍走软件路径 |
| 软件画面 | `renderer.txt` 为 `cpu` 时核心在 CPU 上按游戏画布合成；`runtime/window.c` 经 vita2d 上传为纹理，由 GPU 按显示设置缩放到 960×544（最近邻或线性），宿主指针也在这里绘制。截图读回使用最后一帧 |
| 主频 | 启动即请求应用最高频率（CPU 444 MHz、总线与 GPU 222 MHz），主线程固定在核心 0 |
| 内存 | newlib 堆 256 MiB，SFO 设 `ATTRIBUTE2=12` 取得扩展内存预算；主线程栈 4 MiB |
| 音频 | `cmvs-audio` 的 `mixer` 模式，专用线程在核心 1 以 48 kHz 立体声写入 BGM 端口，两块 1024 帧交替，最多约 43 ms 排队 |
| 文本输入 | 系统输入法对话框（日文 / 英文），上限与 Switch 相同；Shift-JIS 无法表示的字符以消息框提示后重新打开输入法 |
| 菜单与错误页 | 与 Switch 共用 `cmvs-console`，CPU 绘制（`ab_glyph`），按 720 行布局缩放到屏幕 |
| 文件 | 系统重命名不覆盖已有文件，核心先删除旧文件再改名（与 Switch 相同，非原子）；单进程环境下不加用户数据锁 |

输入映射、列表、错误页与输入法编码的逻辑可在开发机上 `cargo test -p cmvs-console -p cmvs-vita`。`menu_preview` / `error_preview` 两个忽略测试设 `CMVS_MENU_SCREEN=vita` 时按 960×544 渲染。

## 测试状态

2026-11-08，Vita3K（Linux AppImage，SDL 前端）：

| 项目 | 结果 |
|---|---|
| 工具链诊断程序 | 文件、线程、64 MiB 堆与 SHA-256 通过 |
| 《アマツツミ》 | 识别、加载、开场画面与系统字体文字通过 |
| 错误页 | 系统字体、缩放布局与按系统设置显示的确定键通过 |
| GPU 画面 | 《アマツツミ》Purple 标志、标题画面与正文文字由 GXM 着色器绘制，显示正确，无 GXM 错误；`cpu` 设置仍走软件路径 |

未验证：真机（含帧率与 GPU 路径：Vita3K 对颜色面步长、无深度面的场景等比硬件宽松）、GPU 与 CPU 的逐像素对比、遮罩过渡与屏幕遮罩的 GPU 路径、按键与触摸实际操作、游戏列表、输入法、消息框、存档与音频听感。
