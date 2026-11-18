# 研究与验证约定

区分格式事实、具体构建的指令语义和宿主策略。文档只保留维护实现所需的身份、契约和支持边界。

## 证据与实现

- 需要保留的地址绑定可执行文件 SHA-256、架构和 image base；不同游戏或不同更新版本重新确认，不能只因 opcode 相同就沿用结论。

- 指令记录包含参数来源、栈回收、返回标志、状态影响、失败行为和已验证范围。尚未确认或实现的指令显式报错。

- 平台无关语义留在 Rust 核心，原生系统接口留在后端。参考项目须记录固定提交与许可，不直接复制未明确授权的源码，见[参考与许可](references.md)。

- 测试仓库仅含合成数据。解码算法修改后，用仓库外真实样本与独立 Python 对照，保留对照范围与结果；需要重现具体样本时再记录输入身份。

- 验证记录保留范围、方法、结果和限制。已解决的断点、逐轮测试数量、本机日志路径和派生比较报告由 Git 历史保存。引擎档案引用的语义记录及有效契约依据继续维护。

- 原游戏保持只读；保存实验使用游戏目录外的新文件或独立用户目录。原始素材、解包剧本、真实存档和 IDA 数据库不进入 Git。

## 新游戏接入

1. 在 `docs/games/<游戏标识>/` 建立档案，参考[模板](games/TEMPLATE.md)，在[游戏目录](games/README.md)登记。游戏标识使用稳定的小写 ASCII 名称。
2. 记录作品、版本/补丁、平台、架构、编码、二进制与关键输入哈希；不知道的字段写“未确认”。同款不同版本先在档案内分节，证据较多时再按版本建子目录。用 `cmvs-profile identify` 获取哈希、PE 时间戳、链接器版本与 switch 信息；共用已登记构建时只需添加游戏记录，新构建则用 `cmvs-profile table --base` 生成派生档案（命令默认 `unreviewed`），登记到 `crates/cmvs-profile-research/profiles/`。见[引擎档案层](design/engine-profiles.md)。
3. 先核对容器签名、格式分支与脚本结构，再测试启动、标题、剧情、选择、片尾和保存/恢复。现有工具支持的格式之外保留明确错误，不跳过未知指令。
4. 分开报告结构检查、VM 记录重放、画面检查、原生窗口/音轨、存档和平台验证。只保留复现所需的参数和输入状态；失败停点只记录尚未解决的问题。
5. 把通用实现说明更新到架构、格式或后端文档；游戏专属差异和验证默认集中在一个 README；游戏目录只保留简要索引，不重复维护详细矩阵。

## 外部样本审计

从仓库根目录运行。以下示例使用 Unix shell，读取独立脚本及归档中的 `.ps3` 条目，并校验所读条目的资源校验和。归档须能用默认参数读取。可省略 `CMVS_SCRIPT_TEST_REPORT`，仅做断言。

```sh
mkdir -p local
CMVS_SCRIPT_TEST_INPUTS='game/data/pack/script.cpz:game/data/pack/start.ps3' \
CMVS_SCRIPT_TEST_REPORT=local/script-sites.tsv \
  cargo test --offline --release -p cmvs-core --test integration \
  script_validation::external_script_structure -- --ignored --exact --nocapture
```

PowerShell 中用 `$env:变量名 = '值'` 设置同名变量，输入路径间改用分号，运行后用 `Remove-Item Env:变量名` 清除。独立脚本或失败条目可能在报告中带有本机路径，报告留在仓库外或忽略的 `local/`。

需要查看运行时对各命令的处理结果时，可另行生成分派报告。输出文件须是新路径。

```sh
CMVS_COMMAND_INVENTORY=local/dispatch.tsv \
  cargo test --offline --release -p cmvs-core --test integration \
  command_inventory::command_inventory -- --ignored --exact
```

汇总时分别记录结构、命令入口和执行地址覆盖。路线覆盖只汇总成功结束的路线；条件分支和游戏完成度需要各自的证据。

## 检查

检查命令统一见[开发检查](architecture.md#检查)；默认忽略的外部样本和硬件测试需按验证目的显式执行，不能计为默认通过。
