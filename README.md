> [!WARNING]
> 项目处于开发中，尚未实现完整 CMVS 游戏兼容。

# cmvs-rs

以 **Rust** 研究并重实现 CMVS，目标是获得全平台支持。从《アマツツミ》开始，逐步扩展到其他引擎版本与作品。平台无关核心优先，窗口、字体、音频和物理输入由后端提供，不新增 C++ 运行核心。

## 当前状态

截至 2026-10-30：已有无第三方依赖的核心、研究 CLI、Windows 桌面预览及可选原生音频。支持已核对分支的脚本、资源、场景、文字、输入、系统存档与剧情存读档处理。

| 对象 | 已验证范围 | 主要缺口 |
|---|---|---|
| 《アマツツミ》PC 样本 | 88 个脚本来源通过结构检查；五条结局选择链及 OP 在记录重放中回到标题；鉴赏四页入口、首张CG、角色切换、滚动/惯性及部分H回想记录重放；部分原生窗口与音频实测，含鉴赏音乐切曲/停止的设备消费检查 | 鉴赏完整交互及全部原生输出未验证；剧情存读档（含游戏内工具栏与存读档画面）及回看已在无窗口重放中验证；系统设置画面与自动送页未实现，原生窗口内的界面操作、全分支和原生完整路线未验证 |
| 《クロノクロック》PC 样本 | 仅摸底：CPZ6 封包，86 个来源通过结构检查；已建引擎档案，研究放行下无窗口 trace 经标题进入オートデモ | 712 个命令未复核，默认在首个系统命令停止；研究放行下停在 `0x2026` |
| 其他 CMVS 作品与版本 | 尚未建立测试档案 | 需要独立确认版本、格式和指令语义 |
| macOS、Linux、Android、iOS | 平台目标 | 尚未完成构建与行为验证 |

记录重放、结构检查、画面抽查和原生运行分别记录，不能相互替代。详细范围与证据见[游戏测试目录](docs/games/README.md)及[《アマツツミ》档案](docs/games/amatsutsumi/README.md)。

## 开始使用

需要 Rust **1.89+ stable**；在项目根目录运行。`--offline` 要求依赖已在本机缓存。

```powershell
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

```powershell
cargo fmt --check
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
```

测试仅含合成数据；修改解码算法后另用仓库外真实样本与独立 Python 对照。涉及后端或 feature 时，追加 [Workspace](docs/workspace.md) 中的全功能检查。
