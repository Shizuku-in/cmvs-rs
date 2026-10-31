# 文档导航

项目能力概览见[项目首页](../README.md)。这里按用途组织文档；具体游戏的兼容性以对应游戏档案为准。

## 使用与开发

| 文档 | 用途 |
|---|---|
| [运行指南](guides/running.md) | 桌面预览、字体、音频、系统存档与显式运行策略 |
| [CLI 指南](guides/cli.md) | 脚本、归档、图像、影片与存档检查工具 |
| [架构](architecture.md) | 平台无关核心、后端契约、运行编排与错误策略 |
| [Workspace](workspace.md) | crate 划分、feature 与检查命令 |
| [研究约定](research.md) | 证据绑定、真实样本验证与新游戏接入 |
| [参考与许可](references.md) | 固定参考提交、依赖许可与使用边界 |
| [引擎档案层设计](design/engine-profiles.md) | 多构建识别、命令状态闸门、全系列普查与分阶段计划 |

## 格式与后端

| 分类 | 文档 |
|---|---|
| 图像 | [PB3 / JBP1](formats/pb3-format.md) |
| 影片 | [JBPD](formats/jbpd-format.md)、[CMV6/8/9 场景影片](formats/cmv9-format.md) |
| 存档 | [CSV2](formats/save-format.md)、[CSS1](formats/system-save-format.md)、[引擎快照](formats/snapshot-format.md) |
| 平台 | [宿主接口](backends/platform-host.md)、[桌面预览](backends/desktop.md)、[wgpu](backends/wgpu.md) |
| 输入与输出 | [动作输入](backends/input.md)、[字体](backends/font-output.md)、[原生音频](backends/audio-output.md) |

格式与后端文档描述已实现分支；其中保留的真实样本和阶段验证目前来自《アマツツミ》，不表示所有 CMVS 版本通用。

## 游戏测试与证据

[游戏测试目录](games/README.md)汇总各款游戏与版本：[《アマツツミ》](games/amatsutsumi/README.md)是主线，[《クロノクロック》](games/chronoclock/README.md)已开始逐条语义研究。其目录保存二进制身份、VM 语义证据、研究记录、审计报告和开发历史。

《アマツツミ》审计后续命令实现状态见[21 条指令推进表](games/amatsutsumi/semantics-progress.md)。

## 归档规则

- 项目首页只保留目标、当前能力、主要限制和入口。日常命令放 `guides/`。
- 通用模块职责放架构文档；格式布局放 `formats/`；系统接口与设备行为放 `backends/`。
- 游戏专属路径、地址、版本差异、路线、覆盖率和实测结果放 `games/<游戏标识>/`。新游戏使用[档案模板](games/TEMPLATE.md)，先登记身份，再记录测试结果。
- 游戏档案 `README.md` 维护最新状态；`research.md` 记过程，`vm-semantics.md` 记逐条语义，覆盖报告保留采集时快照。历史结论不随新进展回写。
- 参考仓库与公共依赖统一登记在 [references.md](references.md)；游戏专属来源说明放对应档案。不要反复向项目首页追加研究流水。
- 仓库只保存说明、哈希和必要元数据；真实游戏、探针输出和 IDA 数据库留在仓库外或忽略的 `local/`。文档内相对链接相对于文档目录，命令均从项目根目录执行。
