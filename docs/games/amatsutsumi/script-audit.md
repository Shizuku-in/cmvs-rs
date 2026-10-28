# 脚本审计方法与缺口

2026-10-29，基线 `d4db050`。初次采集使用临时检查 CLI；现已将结构检查迁入测试接口，删除独立 `cmvs-script-audit` 二进制，Python 汇总器保留在忽略的 `local/`。未改变 VM、资源解码或后端语义。逐来源、逐路线数量和历史采集哈希见 [覆盖报告](script-coverage.md)。

## 结论与边界

`script.cpz` 的 87 个脚本及实际启动使用的包外 `start.ps3`，共 88 个来源全部通过 PS2A 解码、完整线性指令解码、入口/标签/直接跳转目标边界检查。87 个归档资源的数据校验和通过；CPZ 头部/索引摘要仍未验证。包内与包外 `start.ps3` 的代码长度分别为 4752/4726 字节、指令数为 335/330，两者不能合并归因。

重新运行 `[1]`、`[2,1]`、`[2,2,1]`、`[2,2,2,1]`、END1 后的 `[2,2,2,2]`，全部完成保存并到达标题等待点；另从标题空闲进入 OP 并自动回到标题。最后用既有 END2 系统存档副本点击「鑑賞モード」，复现旧 `0x0105` 求值器错误；后续实现及新停点见下表与 VM 语义的“鉴赏入口数据数组”章节。本轮只有 Ctrl、记录文字/音频、16ms 显式帧推进和无窗口执行，不加载原生音轨、不检查画面。

88 个来源中，87 个有成功服务的 PC 记录；未覆盖来源为包内备用 `start.ps3`。`omake.ps3` 只有进入后的少量指令通过，不能算作鉴赏模式通过。其他来源也仅表示执行了其中一部分指令，各脚本携带的公共库仍有大量未调用部分。

静态共有 2,957,772 条顶层指令，本轮成功服务 1,055,703 个去重 PC（35.69%），涉及 431 种静态系统命令中的 250 种。“服务”包括保持原 PC 的等待，不等于完成。未采集条件跳转两侧结果，**没有分支覆盖率**；地址比例不能表示游戏完成度。鼠标、Enter、随机种子变化、系统设置/读取剧情存档/鉴赏内部菜单、不同通关组合及其他宿主环境均不能从这轮 Ctrl 数据推断通过。

## 已确认的未实现项与补测优先级

以下是有当前源码证据且确实出现在静态语料中的缺口，并非完整指令实现清单。

| 优先级 | 缺口 | 证据与当前行为 |
|---|---|---|
| 1 | 鉴赏辅助文本命令 `0x2383` | `0x2380` 默认字体构造和独立所有权已实现，当前 4835 步到 `omake.ps3 / PC=0x4706a / 0x2383`；逻辑图集仅首单元像素已知。具名字体、后续文字内容/绘制仍未实现，终点仍为黑屏 |
| 2 | 完整剧情恢复 `0x22b5` | 静态共 5 处：`main.ps3:0x395de`；`intproc.ps3:0x34752/0x349e8/0x858b8/0x85a3c`。当前完整快照/恢复未实现，路线重放显式绕过启动自动恢复。应独立验证保存、关闭、重启、恢复后的完整状态，不能用 CSS1 持久区代替，见 [快照](../../formats/snapshot-format.md) |
| 3 | 文字对象字体命令 `0x2106` | 静态两处 `intproc.ps3:0x6ec8a/0x7a266`。当前 `MessageHost::try_handle` 不处理 `0x106`；本轮未执行这些位置，不猜测所属菜单或调用条件 |

覆盖报告另列出 181 种本轮未服务的系统命令。这个集合包含可能已实现的命令，不能整体贴上“未知/未实现”标签；执行过的命令也可能有未实现的参数/模式。`0x0105` 已补齐，下一步核对新停点 `0x2383` 并继续鉴赏入口，再逐项研究候选调用路径；未知语义仍保留明确错误。

本节脚本地址绑定覆盖报告所列 `script.cpz` / 包外 `start.ps3` SHA-256；原版语义地址只绑定 `cmvs64.exe` SHA-256 `dc74d2001387ae1aefbdc4204216de5ebb9d3896009280720c4e7a91b42d84f5`。本轮未新增 opcode 语义推断。

## 重复运行

单脚本接口位于 `tests/support/script_validation.rs`：`validate(&Script) -> Result<ScriptStructure, DecodeError>`，返回指令边界、代码/标签数量和系统命令频次。它不依赖路径、归档、游戏名或输出协议，不向运行时核心增加仅供测试的 API。`tests/script_validation.rs` 用合成数据验证接口，并提供默认忽略的外部样本适配器。

从项目目录显式执行外部样本检查（游戏输入只读，报告仅含元数据）：

```powershell
New-Item -ItemType Directory -Force local | Out-Null
$env:CMVS_SCRIPT_TEST_INPUTS = "E:/Download/AMATSUTSUMI/data/pack/script.cpz;E:/Download/AMATSUTSUMI/data/pack/start.ps3"
$env:CMVS_SCRIPT_TEST_REPORT = "local/script-validation-sites.tsv"
cargo test --offline --release --test script_validation external_script_structure -- --ignored --exact --nocapture
Remove-Item Env:CMVS_SCRIPT_TEST_INPUTS, Env:CMVS_SCRIPT_TEST_REPORT
```

`CMVS_SCRIPT_TEST_INPUTS` 支持任意数量的 CPZ6 包与独立 PS2A 文件，路径列表采用宿主平台的 `std::env::split_paths` 规则（Windows 用分号，Unix 用冒号）。`.cpz` 输入校验资源校验和并检查其中 `.ps3` 条目，其他包内资源明确 skip；其他路径作为独立 PS2A 输入。每项失败记录后继续后续检查，最终由 Cargo 测试状态报告失败；零脚本、缺少路径、损坏归档也不能算通过。不设置报告变量时仅做断言，不产生输出文件；设置时用 create_new 拒绝覆盖。报告保留旧 TSV 的 script/system/site/failure/summary 行，便于与已有 PC 元数据比对；重定向 Cargo 标准输出不能代替报告文件。

`local/script_coverage.py` 合并静态 `--sites` 输出和明确分类的 PC 日志，输出到新 Markdown 文件，拒绝覆盖。`--run <标签> <结束日志> <PC日志>` 必须有最终的 `RESULT success=true`；`--partial-run` 只接受显式错误结果、保留失败说明，不接受步数耗尽。运行时 `start.ps3` 必须用 `--startup-key` 映射到确切包外来源。工具拒绝边界外 PC、重复来源、缺失静态记录或结果后追加错误；合成测试命令为 `python local/test_script_coverage.py`。

真实探针、日志、PC 列表、存档副本、TSV 和二进制仍留在忽略的 `local/`，不进入 Git。报告中的前缀分别为 KOKORO=`audit-first2`、KYOKO=`audit-two`、AI=`audit-three`、HOTARU1=`audit-end1b`、HOTARU2=`audit-end2`、OP=`audit-op`、OMAKE=`audit-omake2`，各配同前缀 `.log` 和 `-pcs.tsv`。报告绑定文件、实际探针二进制、最终探针源码和游戏输入哈希。

探针每 100000 次调用替换记录音频后端，避免单后端 4096 请求上限，不修改通道状态。固定选择后按住 Ctrl，选择支点击选项中心，非 staff 脚本空闲 300 个让出后注入一次确认，步数因此不要求与历史画面探针一致。五条路线使用独立新目录，END2 引入既有 END1 文件副本。`audit-replay.exe` 用于こころ/響子/愛/END2，`audit-final.exe` 用于修正单选项检测后的 END1，`audit-targeted.exe` 用于 OP，`audit-extra.exe` 用于失败退出前显式 flush 的鉴赏记录。

以下无效采集均排除：`audit-first` 达到记录音频请求上限；`audit-end1` 因探针只检测两项以上按钮组而漏掉第四次单选项，耗尽预算；旧 `audit-omake` 失败前未 flush PC 缓冲，汇总工具拒绝截断记录，已用 `audit-omake2` 重采。没有用截断记录或探针失败冒充运行时通过。
