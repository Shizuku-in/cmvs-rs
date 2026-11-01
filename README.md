> [!WARNING]
> 项目处于开发中，尚未实现完整 CMVS 游戏兼容。

# cmvs-rs

以 **Rust** 研究并重实现 CMVS，目标是获得全平台支持。从《アマツツミ》开始，逐步扩展到其他引擎版本与作品。平台无关核心优先，窗口、字体、音频和物理输入由后端提供，不新增 C++ 运行核心。

## 当前状态

截至 2026-10-31：已有无第三方依赖的核心、研究 CLI、Windows 桌面预览（默认 CPU，可选 wgpu 合成）及可选原生音频。支持已核对分支的脚本、资源、场景、文字、输入、系统存档与剧情存读档处理。

| 对象 | 已验证范围 | 主要缺口 |
|---|---|---|
| 《アオイトリ》PC 样本 | CPZ7，86个来源结构检查；显式研究放行下到标题5按钮，启动确定性通过；进入首条剧情 | 多数命令未复核；比例ruby注音、部分页面、剧情快照与原生输出待推进；文字分派仅迁移一部分，见[档案](docs/games/aoitori/README.md) |
| 《アマツツミ》PC 样本 | 88 个脚本来源通过结构检查；五条结局选择链及 OP 在记录重放中回到标题；鉴赏四页入口、首张CG、角色切换、滚动/惯性及部分H回想记录重放；剧情存读档、回看、系统设置三页、音频配置及自动送页经无窗口重放验证；部分原生窗口与音频实测，含首条真实语音后的自动推进及合成语音压低BGM/恢复 | 系统设置其余修改与完整重置未完成；已实现设置支持独立用户目录持久化，原生退出重启待验证；更多原生自动送页路径、鉴赏完整交互、原生窗口内的界面操作、全分支和原生完整路线未验证 |
| 《クロノクロック》PC 样本 | CPZ6封包，86个来源通过结构检查；引擎档案已有455条完整指纹继承、167条推定、25条人工复核；默认闸门下无快照runner进入オートデモ | 65条命令未复核，runner停在`0x22b2`；部分已复核命令尚无处理器，剧情快照格式未实现，未验证原生输出与完整剧情 |
| 其他 CMVS 作品与版本 | 尚未建立测试档案 | 需要独立确认版本、格式和指令语义 |
| macOS、Linux、Android、iOS | 平台目标 | 尚未完成构建与行为验证 |

记录重放、结构检查、画面抽查和原生运行分别记录，不能相互替代。详细范围与证据见[游戏测试目录](docs/games/README.md)及[《アマツツミ》档案](docs/games/amatsutsumi/README.md)。

## 开始使用

需要 Rust **1.89+ stable**；在项目根目录运行。`--offline` 要求依赖已在本机缓存。

```bash
cargo run --offline -- "<游戏目录>/data/pack/start.ps3"
cargo test --offline
```

桌面预览、启动快照屏障、字体、音频和独立用户目录配置见[运行指南](docs/guides/running.md)。检查工具见[CLI 指南](docs/guides/cli.md)。所有游戏输入保持只读，素材、解包剧本、真实样本和 IDA 数据库不进入 Git。

## 文档与开发

- [文档导航与归档规则](docs/README.md)：按用途查找资料。
- [架构](docs/architecture.md)与 [Workspace](docs/workspace.md)：模块职责、依赖和完整检查命令。
- [研究约定](docs/research.md)：证据要求及新游戏接入步骤。
- [引擎档案层](docs/design/engine-profiles.md)：按构建选择命令语义的闸门，以及 2007–2023 全系列普查。
- [参考与许可](docs/references.md)：固定提交、第三方依赖和复用边界。
- [开发历史](docs/games/amatsutsumi/history.md)：原 README 的阶段记录与旧停点。

提交前运行：

```bash
cargo fmt --check
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
```

测试仅含合成数据；修改解码算法后另用仓库外真实样本与独立 Python 对照。涉及后端或 feature 时，追加 [Workspace](docs/workspace.md) 中的全功能检查。

## 许可证

[MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE) 

本项目许可证不授予原游戏程序、素材或脚本的使用与分发权。
