# 文档导航

## 使用

- [运行指南](guides/running.md)：桌面运行、字体、音频、存档与运行策略。
- [CLI 指南](guides/cli.md)：脚本、资源和存档检查工具。
- [Web](../platforms/web/README.md)、[Switch](../platforms/switch/README.md)与 [PS Vita](../platforms/psvita/README.md)：各平台的构建、操作与验证范围。

## 开发

- [架构](architecture.md)与 [Workspace](workspace.md)：模块职责、依赖、feature 和检查命令。
- [分派开发指南](guides/dispatch.md)：系统命令入口、处理器约定与测试。
- [引擎档案](design/engine-profiles.md)与[契约比较](design/command-contracts.md)：构建识别、语义证据和命令闸门。
- [通用游戏探针](design/probe.md)：路线、输入、页面、存读档及设置验证。
- [格式文档](formats/)与[后端文档](backends/)：数据布局、渲染、输入、字体和音频接口。

## 游戏与证据

- [游戏档案目录](games/README.md)：各游戏的样本身份、支持范围与验证结果。
- [研究约定](research.md)：证据要求及新游戏接入流程。
- [参考与许可](references.md)：参考实现、依赖与复用边界。
- [发布说明](release-notes.md)：行为变更及迁移说明。

## 归档约定

- 通用职责放架构文档，格式放 `formats/`，系统接口放 `backends/`；游戏专属地址、版本差异和实测结果放 `games/<游戏标识>/`。
- 游戏档案保留当前结论、支持边界和必要证据；覆盖报告注明采集基线与输入哈希，不将实现变化计为新增验证。
- 真实游戏、探针输出和 IDA 数据库留在仓库外或忽略的 `local/`。文档相对链接以所在目录为准，命令从项目根目录执行。
