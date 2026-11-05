# Switch Homebrew（开发中）

当前是工具链验证阶段，尚不能运行游戏。`cmvs-switch-smoke.nro` 仅显示
启动信息、轮询手柄，并在按下 + 时退出；它已链接 CMVS 核心，并调用 SHA-256 验证、Rust std 目录遍历、文件读取和时钟探针；尚无游戏运行循环。

## 构建验证

从仓库根目录执行：

```sh
docker build -t cmvs-switch-build platforms/switch
docker run --rm -v "$PWD:/src" -w /src cmvs-switch-build sh platforms/switch/check-core.sh
docker run --rm -v "$PWD:/src" -w /src cmvs-switch-build sh platforms/switch/build-smoke.sh
```

产物：`platforms/switch/build/cmvs-switch-smoke.nro`。复制到 SD 卡的
`switch/cmvs-rs/` 后从 hbmenu 启动。编译成功不能替代真机验证。

核心检查使用独立 Horizon/newlib target 和 nightly-2026-01-20 的
`-Zbuild-std`。这只是编译检查，不代表 std 文件系统 ABI 已验证；
`prepare-libc.sh` 为 Switch 构建创建私有 libc 副本并修正设备号、inode 和块计数类型；`runtime/abi.c` 检查 C 侧对应类型与关键偏移。该副本仅用于本 target，不应用于其他 newlib 平台。
桌面默认构建不受此 target 影响。

## 后续接入

1. 核心静态库链接和 std 文件读取、目录、时钟验证。
2. 在 cmvs-native 集中封装 libnx FFI；新增安全的 Switch 运行宿主。
3. 接入 CPU 画面呈现、字体、手柄/触摸、完整帧边界。
4. 接入 PCM 音频输出、独立存档目录与退出保存。
5. 真机验证与性能分析，再决定是否引入 deko3d。

## 参考

工具链路线参考 [siglus_rs Switch port](https://github.com/xmoezzz/siglus_rs/tree/5f358ef1360b0439e92aed2b749e8f25d8721954/platform/switch)
（MPL-2.0）：自定义 Horizon/newlib target、build-std、C/libnx 宿主。
本目录未复制该项目的宿主或补丁源码。

## 已执行验证（2026-11-05）

- Docker 镜像构建成功（devkitPro 基础镜像已固定 digest）。
- `check-core.sh` 成功：现有 `cmvs-core` 无修改通过 Horizon/std 交叉检查。
  nightly 的 std 有 future-incompatibility 警告，不影响本次检查。
- `build-smoke.sh` 成功：生成约 157 KiB 的 NRO，C 编译启用 `-Werror`。
- 尚未真机运行；尚未验证 Rust 静态库链接、文件系统 ABI 和游戏运行。

## Rust 核心链接探针

运行 `build-smoke.sh` 会先构建 `cmvs-native` 静态库，再与 libnx 链接。
FFI 导出集中在 `cmvs-native`，核心无需 unsafe。请将产物保持原名放到
`sdmc:/switch/cmvs-rs/cmvs-switch-smoke.nro`：探针读取该文件的 NRO0 标记和
文件长度，列出同目录最多 16 个条目，检查核心 SHA-256 已知向量并打印耗时。
成功显示 `Rust/core probe status: 0`，失败显示原因及状态 1；探针不写入 SD 卡。

构建日志中 workspace 的 libc patch unused 警告不表示 std 未使用补丁：
应同时看到 `Compiling libc v0.2.178 (/src/platforms/switch/build/libc-0.2.178)`。
该版本属于 nightly 的 build-std 依赖，workspace 自身使用其他 libc 版本。

已验证 Rust 核心静态库、C ABI 断言和最终 NRO 链接成功；桌面
`cargo check -p cmvs-native --locked` 通过。探针运行结果仍待真机确认。
