# CLI 指南

在仓库根目录运行。把示例路径换成实际路径，含空格时加引号。下面的命令要求依赖已缓存，首次构建可去掉 `--offline`。

`cargo run` 的 `--` 后是工具参数。把示例中 `--` 后的参数换成 `--help`，可查看工具的完整用法。各二进制及构建特性见[工具清单](../../crates/cmvs-cli/README.md#binaries)，播放游戏见[运行指南](running.md)。

## 脚本与执行跟踪

```sh
cargo run --offline --bin cmvs-inspect -- game/data/pack/start.ps3
cargo run --offline -p cmvs-disassembler -- game/data/pack/start.ps3
cargo run --offline --bin cmvs-trace -- game/data/pack/start.ps3 --steps 10000
```

- `cmvs-inspect` 查看 PS2A/PSS0 的头部、分段和原始文本字节。用 `--payload <新文件>` 导出解码负载。
- `cmvs-disassembler` 按指令顺序反汇编并统计系统命令。
- `cmvs-trace` 在步数预算内执行脚本。可用 `--profile <档案ID或游戏别名>` 显式指定档案。以归档为输入时，用 `--script <条目名>` 选入口，用 `--script-archive <归档>` 启用脚本切换。

导出译文表后，在需要修改的行末添加一个制表符和译文。前三列依次为段名、原始十六进制字节偏移和原文。省略第四列保留原文，第四列留空表示替换为空串。导入时校验源脚本 SHA-256。数据段译文须与原文编码后的字节数相同。

```sh
cargo run --offline -p cmvs-disassembler -- scene.ps2 --export-text scene.tsv --source-encoding shift-jis
cargo run --offline -p cmvs-assembler -- script import-text scene.ps2 scene.tsv edited.ps2 --target-encoding shift-jis
```

跟踪启动流程时，可载入启动配置、脚本包和存档元数据，并推进虚拟帧时钟：

```sh
cargo run --offline --bin cmvs-trace -- game/data/pack/start.ps3 \
  --bootstrap --script-archive game/data/pack/script.cpz \
  --save-dir game/save --system-save game/save/system.dat \
  --record-platform game --frame-ms 16 --steps 1000000
```

trace 默认停在快照点。仅需继续观察执行时可加 `--continue-without-snapshot`。脚本回调时钟由 `--virtual-tick` 单独指定。命令状态和阻止策略见[引擎档案](../design/engine-profiles.md#选择与闸门)。

### 编辑脚本项目

```sh
cargo run --offline -p cmvs-disassembler -- scene.ps2 --profile asukimi --export scene.project
cargo run --offline -p cmvs-assembler -- script assemble scene.project edited.ps2
```

`cmvs-script-v1` 是 UTF-8 制表符分隔文件。首行依次记录容器、版本、密钥、保留字段、压缩方式、档案、字符串编码和原代码段、数据段字节数。数值采用十六进制。汇编默认沿用原压缩方式，可用 `--compressed` 或 `--uncompressed` 覆盖。

- 每行 `code` 定义一个符号和一条指令。保留原指令的符号，给新增指令起唯一的名字。操作数 `@名称` 指向指令，`&名称` 和 `%名称` 分别引用 16 位和 32 位标签索引，`$名称` 引用文本。行尾 `#` 列是注释。
- `entry` 指向入口指令。`label` 行按出现顺序建立标签表。`text` 行按顺序拼接字节，`\x00` 表示终止符，共享后缀可单独引用。`data` 行须保留原偏移与字节数，可用 `append<Tab>十六进制字节` 在数据段末尾追加。
- 移动代码后，汇编器按档案处理回调。若报告动态回调，先将产生目标地址的常量写成 `@名称`，或调整运行时计算，再给注册指令添加 `runtime-pc` 列（位于注释前）。改变代码布局后，旧存档中的指令地址可能失效。

## 归档与资源

```sh
cargo run --offline --bin cmvs-cpz -- --list game/data/pack/script.cpz
cargo run --offline --bin cmvs-cpz -- --extract game/data/pack/script.cpz main.ps3 main.ps3
```

`cmvs-cpz` 只传路径时查看归档头部，`--script <归档> <条目名>` 查看解码后的脚本。条目名须为 ASCII。列表会显示资源校验和；提取和脚本检查也不验证资源校验和。

CPZ5 自动查找相邻的 `start.ps3` 或 `start.ps2`，CPZ7 查找 `start.ps3`。没有启动脚本时，CPZ5 从索引推断模式，CPZ7 使用零参数。可在操作模式前指定 `--start-script`、`--packing-mode`（CPZ5）或 `--keys`（CPZ7）。支持范围见 [CPZ 格式](../formats/cpz-format.md)。

用列表中的条目编号将编辑好的文件写入新包。以下示例将 PNG 编为 PB3B type 1 并替换第 0 项。

```sh
cargo run --offline -p cmvs-assembler -- pb3 encode edited.png edited.pb3
cargo run --offline -p cmvs-assembler -- cpz replace chip.cpz new.cpz 0 edited.pb3
```

六种 CPZ 都支持按条目编号替换、导出清单和按清单新建。导出目录中的 `archive.tsv` 保留目录边界、重名条目和原始名称；名称按 Shift-JIS 或 `\xNN` 转义，输入路径相对于清单，解析后须位于清单目录内。CPZ3/5/6 的运行时文件名限 63 字节。

```sh
cmvs-assembler cpz export script.cpz archive --profile realive
cmvs-assembler cpz pack archive/archive.tsv new.cpz --profile realive
```

手写清单时，首行为 `cmvs-cpz-v1<Tab>cpz7`，文件行为 `file<Tab>名称<Tab>路径`。目录型封包用 `dir<Tab>目录名` 开组。CPZ5 可用 `mode` 指定 5～9（默认 9），CPZ5/7 可用 `encryption` 指定 0 或 1（默认 1）。CPZ7 的 `parameters` 行按登记顺序填写四个十六进制整数，默认全零。各列用制表符分隔。独立的 CPZ7 文件可在导出或替换时用 `--keys k0,k1,k2,k3` 指定四个十进制或 `0x` 前缀的参数。`pack` 回读全部明文，`replace` 回读替换条目，加 `--verify-all` 也会解密比较未改条目。

CVNS 静态 PB2 图像可导出为 PNG，再用 `--type 1|2` 和 `--bpp 24|32` 指定写回格式。默认写成 type 2、32 位。保留源格式时指定原类型和位深，24 位要求所有像素完全不透明。type 3/4 的 JBP1 源图也可导出并转换为这两种无损格式。

```sh
cargo run --offline -p cmvs-assembler -- pb2 decode logo.pb2 logo.png
cargo run --offline -p cmvs-assembler -- pb2 encode logo.png edited.pb2 --type 1 --bpp 24
```

CVNS 的 MSK0 转场遮罩和字形图集可导出为 8 位灰度 PNG。编辑时保留图集尺寸和字格位置，每个灰度值对应一个原始强度值。

```sh
cargo run --offline -p cmvs-assembler -- msk decode filter.msk filter.png
cargo run --offline -p cmvs-assembler -- msk encode filter.png edited.msk
```

PB2 动画与命名部件用 PNG 集合和 `images.tsv` 编辑。PB3 差分图用底图的解码结果和引用名写回。CUR 项目逐行保存 XOR/AND 原始字节，表头记录位深和热点，调色板保留原始顺序。

```sh
cmvs-assembler pb2 export motions.pb2 frames
cmvs-assembler pb2 pack frames/images.tsv new.pb2
cmvs-assembler pb3 encode composite.png new.pb3 --type 8 --base base.png --reference base
cmvs-assembler cursor decode pointer.cur pointer.tsv
cmvs-assembler cursor encode pointer.tsv new.cur
```

### 整包增量构建

```sh
cmvs-assembler build export game project
cmvs-assembler build extract project 42 project/edit.png --source game
cmvs-assembler build assemble project new-game --source game
```

在 `project/game.tsv` 中按文件名和条目号找到目标资源，上例以 42 为例。将对应 `asset` 行末列的 `-` 改为 `edit.png`。PB2 集合填写 `images.tsv`，清单及其引用的图片都须位于项目内。CUR 填写光标表，脚本可填写已汇编的容器或由 `cmvs-disassembler --export` 导出的项目。无专用编码器的资源可填写替换文件。底图改动时保留未编辑覆盖层的原始差分；为覆盖层指定 PNG 会基于最终底图重算差分。删除项目的 `cache/` 可重新编码全部编辑项。

源目录提供游戏数据、配置、运行库、可执行文件及启动所用的系统存档模板（存在时）。构建按清单校验源文件 SHA-256，在新目录发布结果。若原始数据未被识别，可在导出时指定 `--profile`。 [档案选择](../design/engine-profiles.md)

| 工具 | 用法 |
|---|---|
| `cmvs-pb3` | 检查 PB3B；`--rgba <新文件>` 导出顶向下的 RGBA |
| `cmvs-cursor` | 检查 CUR；`--rgba <新文件>` 导出 RGBA，带反色像素时可用 `--planes <新文件>` |
| `cmvs-cmv` / `cmvs-mgv` | 检查 CMV6 / MGV1；`--check-frames` 解码检查，`--frame 0 frame.png` 导出首帧 |
| `cmvs-audio` | 检查 Ogg Vorbis、PCM WAVE、MV2X 或 CMV6 音轨；`--pcm <新文件>` 导出 PCM，构建时启用 `--features audio-output` |

## 存档

```sh
cargo run --offline --bin cmvs-save -- --records game/save/save000.dat
cargo run --offline --bin cmvs-system-save -- game/save/system.dat
```

`cmvs-save` 检查 CMVS 的 CSV2 剧情档。可以用 `--vm-state` 验证 VM 字段和主脚本 PC，或用 `--visual-state` 检查支持的文字、窗口和场景记录。`--section <编号> <存档> <新文件>` 导出指定段。`cmvs-system-save` 检查 CSS1 系统档，可加 `--payload <新文件>` 导出负载。游戏内读档与 CVNS 支持范围见[运行指南](running.md#剧情存档)。

## 通用探针

```sh
cargo run --offline --release -p cmvs-probe -- game --out report --font font.ttf
```

默认运行 S1–S5、R1 和 D1，生成 `summary.json` 与 `events.jsonl`。输出目录必须尚未存在，探针在其中创建独立用户目录。用 `--checks R2,R3` 选择检查，或用 `--checks all` 运行全部。完整路线可能耗时较长，可用 `--max-steps` 调整每个会话的预算。

`--fork save` 优先从选项处保存的检查点分支。`--without-snapshots` 跳过快照捕获和 V1–V4，不能与 `--fork save` 合用。检查清单见 `--help`，退出码、报告和纠正清单见[探针说明](../../crates/cmvs-probe/README.md)。

## 引擎档案

```sh
cargo run --offline --release -p cmvs-profile-research -- identify game
cargo run --offline --release -p cmvs-profile-research -- show akifd
```

`identify` 识别构建并核对已登记的分派表。`show` 查看档案状态与命令映射。`table` 输出候选档案，`generate-playback` 生成静态运行表，详见[引擎档案](../design/engine-profiles.md)。

## 外部样本审计

脚本结构、命令入口和执行覆盖的检查方法见[研究与验证约定](../research.md#外部样本审计)。
