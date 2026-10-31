# cmvs-probe

与游戏无关的无头整机检查工具。设计、通用性规则与分阶段计划见[通用游戏探针设计草案](../../docs/design/probe.md)。依赖 `cmvs-core`，并借用 `cmvs-desktop` 的字体模块提供字形度量；不打开窗口或音频设备。

主要职责：

- 通过档案层识别构建，把构建差异表示为能力集合（如 `archive:cpz6`、`key-config`、`snapshot-layout`）；缺少能力的检查记为 `skipped`。
- 在隔离用户目录中，用固定种子和虚拟帧时间运行 `Startup`；输入以抽象动作发出，由鼠标或键盘设备翻译。
- 只用引擎层观察量（消息等待、按钮组、场景渐变、摘要值）判断状态，代码中不出现游戏身份、脚本名或脚本地址；`tests/no_game_constants.rs` 检查这条规则。
- 输出 `summary.json` 与 `events.jsonl`，每个结果注明达到的判定等级（L0/L1/L2）。

从仓库根目录运行：

```bash
cargo test --offline -p cmvs-probe
cargo run --offline --release -p cmvs-probe -- "<游戏目录>" --out "<新输出目录>" --font "<字体文件>"
```

当前实现 S1–S3、R1、D1，以及需在 `--checks` 中显式列出的 R2（选项树探索）和 R3（地址覆盖），详见设计文档第 13、14 节。游戏目录只读；输出目录必须不存在。
