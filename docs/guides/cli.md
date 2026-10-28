# CLI 检查指南

所有命令从项目根目录执行。这里使用 `<游戏目录>` 和 `<新输出文件>` 作为占位符，请替换后运行。工具只支持已核对的格式分支，不能据解析成功推断整款游戏可运行。当前真实样本验证来自[《アマツツミ》](../games/amatsutsumi/README.md)。

## 脚本与执行跟踪

```powershell
cargo run --offline -- "<游戏目录>/data/pack/start.ps3"
cargo run --offline -- "<游戏目录>/data/pack/start.ps3" --payload "<新输出文件>"
cargo run --offline --bin cmvs-disasm -- "<游戏目录>/data/pack/start.ps3"
cargo run --offline --bin cmvs-trace -- "<游戏目录>/data/pack/start.ps3" --steps 10000
```

默认 `cmvs-inspect` 检查 PS2A 头部、分段和解码结果，文本保留原始字节并以十六进制显示。`--payload` 输出不含原始头部，拒绝覆盖。线性反汇编不跟随分支；trace 有步数上限，未知状态和未实现命令显式停止。

完整启动研究示例：

```powershell
cargo run --offline --bin cmvs-trace -- "<游戏目录>/data/pack/start.ps3" --bootstrap --save-dir "<游戏目录>/save" --system-save "<游戏目录>/save/system.dat" --script-archive "<游戏目录>/data/pack/script.cpz" --record-platform "<游戏目录>" --continue-without-snapshot --frame-ms 16 --steps 1000000
```

`--bootstrap` 应用已核对的初始化；`--save-dir` 只读扫描剧情槽，不会自动推断 `--system-save`。记录平台请求不创建窗口，帧时间必须显式提供；省略 `--frame-ms` 时冻结时间并在等待处停止。增加步数不能绕过未知命令、快照或完整恢复屏障。VM 地址及语义见[本作证据](../games/amatsutsumi/vm-semantics.md)。

## 归档与资源

```powershell
cargo run --offline --bin cmvs-cpz -- "<游戏目录>/data/pack/script.cpz"
cargo run --offline --bin cmvs-cpz -- --list "<游戏目录>/data/pack/script.cpz"
cargo run --offline --bin cmvs-cpz -- --script "<游戏目录>/data/pack/script.cpz" main.ps3
cargo run --offline --bin cmvs-cursor -- "<游戏目录>/data/icon/01.cur"
cargo run --offline --bin cmvs-pb3 -- "<PB3 文件>" --rgba "<新输出文件>"
cargo run --offline --bin cmvs-cmv -- "<游戏目录>/data/video/op.cmv"
```

`cmvs-cpz` 支持当前 CPZ6 模式 1，默认检查头部，`--list` 解析索引；CLI 读取不验证资源校验值，运行时图像读取会验证。PB3 的导出为顶向下 RGBA；CUR 的可选 `--planes <新文件>` 导出 AND/XOR 平面，不是 RGBA。具体布局和限制见[格式目录](../README.md)。

## 存档

```powershell
cargo run --offline --bin cmvs-save -- "<游戏目录>/save/save000.dat"
cargo run --offline --bin cmvs-save -- --records "<游戏目录>/save/save000.dat"
cargo run --offline --bin cmvs-save -- --section 0 "<游戏目录>/save/save000.dat" "<新输出文件>"
cargo run --offline --bin cmvs-system-save -- "<游戏目录>/save/system.dat"
```

CSV2 检查与解压不会恢复 VM，段导出拒绝覆盖。CSS1 的模板编码、可选输出与独立用户目录策略见[系统存档](../formats/system-save-format.md)，剧情恢复边界见[快照](../formats/snapshot-format.md)。

## 外部样本审计

可复用的脚本结构测试适配器接受独立 PS2A 与 CPZ6 输入，默认忽略，需显式设置输入路径。当前完整命令、报告协议及边界见[脚本审计方法](../games/amatsutsumi/script-audit.md#重复运行)。测试本身只保存合成数据，真实输入与元数据报告按游戏分开归档。
