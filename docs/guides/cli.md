# CLI 指南

在仓库根目录运行。把示例路径换成实际路径，含空格时加引号。下面的命令要求依赖已缓存，首次构建可去掉 `--offline`。

`cargo run` 的 `--` 后是工具参数。把示例中 `--` 后的参数换成 `--help`，可查看工具的完整用法。各二进制及构建特性见[工具清单](../../crates/cmvs-cli/README.md#binaries)，播放游戏见[运行指南](running.md)。

## 脚本与执行跟踪

```sh
cargo run --offline --bin cmvs-inspect -- game/data/pack/start.ps3
cargo run --offline --bin cmvs-disasm -- game/data/pack/start.ps3
cargo run --offline --bin cmvs-trace -- game/data/pack/start.ps3 --steps 10000
```

- `cmvs-inspect` 查看 PS2A/PSS0 的头部、分段和原始文本字节。用 `--payload <新文件>` 导出解码负载。
- `cmvs-disasm` 按指令顺序反汇编，并统计系统命令。
- `cmvs-trace` 在步数预算内执行脚本。可用 `--profile <档案ID或游戏别名>` 显式指定档案。以归档为输入时，用 `--script <条目名>` 选入口，用 `--script-archive <归档>` 启用脚本切换。

跟踪启动流程时，可载入启动配置、脚本包和存档元数据，并推进虚拟帧时钟：

```sh
cargo run --offline --bin cmvs-trace -- game/data/pack/start.ps3 \
  --bootstrap --script-archive game/data/pack/script.cpz \
  --save-dir game/save --system-save game/save/system.dat \
  --record-platform game --frame-ms 16 --steps 1000000
```

trace 默认停在快照点。仅需继续观察执行时可加 `--continue-without-snapshot`。脚本回调时钟由 `--virtual-tick` 单独指定。命令状态和阻止策略见[引擎档案](../design/engine-profiles.md#选择与闸门)。

## 归档与资源

```sh
cargo run --offline --bin cmvs-cpz -- --list game/data/pack/script.cpz
cargo run --offline --bin cmvs-cpz -- --extract game/data/pack/script.cpz main.ps3 main.ps3
```

`cmvs-cpz` 只传路径时查看归档头部，`--script <归档> <条目名>` 查看解码后的脚本。条目名须为 ASCII。列表会显示资源校验和；提取和脚本检查也不验证资源校验和。

CPZ5 自动查找相邻的 `start.ps3` 或 `start.ps2`，CPZ7 查找 `start.ps3`。没有启动脚本时，CPZ5 从索引推断模式，CPZ7 使用零参数。可在操作模式前指定 `--start-script`、`--packing-mode`（CPZ5）或 `--keys`（CPZ7）。支持范围见 [CPZ 格式](../formats/cpz-format.md)。

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
