# Switch Homebrew（开发中）

实验性软件渲染播放器，支持游戏加载、文字、基础手柄输入与核心存档路径。
尚无音频输出，未经过真机验证；未知游戏及未实现指令会报错。

## 构建

在仓库根目录执行（需要 Docker）：

```sh
docker build -t cmvs-switch-build platforms/switch
docker run --rm -v "$PWD:/src" -w /src cmvs-switch-build sh platforms/switch/build-player.sh
```

## 部署与操作

将 `platforms/switch/build/cmvs-switch.nro` 放到 SD 卡 `switch/cmvs-rs/`，
同目录下放原始游戏目录 `game/` 和日文字体 `font.ttf`（字体索引 0）。
通过 hbmenu 的应用接管模式启动。

方向键移动光标，A/B 对应鼠标左/右键，+ 保存设置并退出。
用户数据位于 `users/<游戏标识>/`，运行错误写入 `error.log`。
暂未接入触摸、音频、系统文字输入和字体选择；HOME 强制退出不保证保存设置。

`build-smoke.sh` 另生成诊断程序 `cmvs-switch-smoke.nro`，可检查文件读取、
核心 SHA-256，并按 A 检查游戏初始化；保持原名放在同一 SD 卡目录。

工具链路线参考 [siglus_rs Switch port](https://github.com/xmoezzz/siglus_rs/tree/5f358ef1360b0439e92aed2b749e8f25d8721954/platform/switch)（MPL-2.0）；未复制其宿主或补丁源码。
