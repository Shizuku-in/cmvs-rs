# 构建 Apple 播放器

在 macOS 上安装 Xcode、XcodeGen、jq 和 Rust stable。以下命令均在仓库根目录运行。添加游戏、选择字体与恢复存档的步骤见[启动器使用指南](app/README.md)。

## 构建应用

首次构建先安装四个 Rust 目标，再生成 macOS、iOS 真机和模拟器共用的 XCFramework。修改 Rust 代码或 C 头文件后，重新运行第二条命令。

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin aarch64-apple-ios aarch64-apple-ios-sim
sh platforms/apple/build.sh xcframework
```

按运行平台选择一条命令。脚本会用 XcodeGen 生成工程，并将应用写到对应目录。

| 运行平台 | 构建命令 | 产物 |
|---|---|---|
| macOS（Intel 和 Apple 芯片） | `sh platforms/apple/build.sh macos --reuse-framework` | `target/apple/macos/CMVS.app` |
| iOS 模拟器（arm64） | `sh platforms/apple/build.sh ios-simulator --reuse-framework` | `target/apple/ios-simulator/CMVS.app` |
| iPhone / iPad 真机 | `sh platforms/apple/build.sh ios --reuse-framework` | `target/apple/ios/CMVS.app` |

Release 流程将 macOS 通用应用打包为 ZIP，将 iOS 真机应用打包为未签名 IPA。安装 IPA 前需使用自己的签名身份和描述文件签名。

## 验证

在 Linux 或 macOS 上运行 `cargo test --locked -p cmvs-apple` 可检查 Rust 桥接层与存档备份恢复。Linux 上的 CPU 冒烟测试已从合成游戏读回非黑的 64×48 逻辑帧。

Apple CI 构建三种应用，运行 Swift 模拟器测试，并检查逻辑帧为 64×48 且非黑屏。模拟器没有 Metal 设备时，冒烟测试使用 CPU 渲染。复现命令见 [Apple CI 步骤](../../.github/workflows/ci.yml)。真机音频中断、触控和存读档仍待验证。
