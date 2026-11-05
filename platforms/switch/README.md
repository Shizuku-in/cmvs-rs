# Switch Homebrew（开发中）

实验性软件渲染播放器，支持游戏加载、文字、Vorbis 音频、手柄与触摸输入及核心存档路径。
未知游戏及未实现指令会报错。

## 构建

在仓库根目录执行（需要 Docker）：

```sh
docker build -t cmvs-switch-build platforms/switch
docker run --rm -v "$PWD:/src" -w /src cmvs-switch-build sh platforms/switch/scripts/build-player.sh
```

## 部署

将 `platforms/switch/build/cmvs-switch.nro` 放到 SD 卡 `switch/cmvs-rs/`，同目录下放原始游戏目录 `game/`。
可选放入日文字体 `font.ttf`（字体索引 0）；缺省时使用系统共享字体（日/美/欧标准字库）。
通过 hbmenu 的应用接管模式启动。

用户数据位于 `users/<游戏标识>/`，运行错误（含脚本名、PC 与步数）写入 `error.log`。
存档时间与桌面、Android 一致按 UTC 记录。

## 操作

游戏动作仍由 `key.cfg` 决定；宿主只把按键映射到引擎输入码：

| 输入 | 引擎输入 |
|---|---|
| 左摇杆 | 移动指针（死区外二次曲线，满偏约 1.2 秒横穿画面） |
| 触摸 | 在画面内按下即为左键并移动指针；黑边起手、第二指不生效 |
| A / B | 鼠标左 / 右键 |
| 方向键 | 方向键（引擎按钮导航，可移动指针） |
| Y / X | Enter / Esc |
| ZR / ZL | Ctrl / Shift |
| L、R、右摇杆上下 | 滚轮上 / 下，按住 400 ms 后每 100 ms 重复 |
| 长按 + 1 秒 | 保存设置并退出 |

从 HOME 菜单关闭软件时，程序先保存设置再退出；若系统拒绝锁定退出（例如相册小程序模式），仍可能被直接终止。
切到 HOME 或覆盖层时输入取消、音频静音，引擎按游戏的后台绘制设置暂停。

## 实现说明

- 画面：1280×720 RGBA 线性帧缓冲，核心 CPU 合成后按显示设置缩放。
- 音频：`cmvs-audio` 的 `mixer` 模式（与桌面同一套声道逻辑，不含 cpal），专用线程在 CPU 核心 1
  上以 48 kHz 立体声、4×1024 帧缓冲写入 audout，最多约 85 ms 排队。
- `runtime/libc.c` 补上 libnx newlib 缺少、而 Rust std 线程与 `HashMap` 需要的 `sysconf`/`getrandom`。
- 输入策略位于 `player/src/controls.rs`，软键盘的编码转换位于 `player/src/text_input.rs`，
  均可在开发机上 `cargo test -p cmvs-switch`。

存档注释等文本输入使用系统软键盘，游戏标题与提示显示在键盘上方；上限沿用桌面宿主（字节上限折算为全角字数，最多 127 字）。
文字按 Shift-JIS 保存（cmvs-font 目前只渲染 Shift-JIS）：无法表示的字符会在键盘内提示并要求修改；
与原版一致，引擎只保留全角字符，全为半角时沿用原注释。键盘关闭时仍按住的按键在松开前不生效。

暂未接入字体选择对话框。

`scripts/build-smoke.sh` 另生成诊断程序 `cmvs-switch-smoke.nro`，可检查文件读取、
核心 SHA-256，并按 A 检查游戏初始化；保持原名放在同一 SD 卡目录。

## 测试状态

2026-11-06 在 Ryujinx 1.3.3（固件 22.5.0，主机模式）运行《アマツツミ》PC 版：启动、系统共享字体文字、
BGM、手柄操作、存档（含同槽覆盖、移动与特殊槽）及长按 + 退出保存设置通过，可正常游玩。
未验证：真机、触摸（需掌机模式）、软键盘（该作无存档注释）、HOME 关闭保存（Ryujinx 退出锁为空实现）。