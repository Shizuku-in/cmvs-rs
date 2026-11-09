# 汉化补丁转换工具

将支持的汉化补丁转换为 cmvs-rs 可读取的脚本和资源。需要原版游戏及对应补丁；工具按 SHA-256 检查输入文件或所用补丁成员的版本。

## 支持的游戏

| 游戏 | 转换与启动说明 | 输出编码 | 封包格式（与原版一致） | 引擎 profile |
| --- | --- | --- | --- | --- |
| 明日の君と逢うために | [asukimi](asukimi/README.md) | UTF-8、GBK | CPZ3 | `cmvs-20071117` |
| 未来ノスタルジア | [miranosu](miranosu/README.md) | UTF-8、GBK | CPZ5 | `cmvs-20110525` |
| ハピメア | [hapymaher](hapymaher/README.md) | UTF-8、GBK | CPZ5 | `cmvs-20130131` |
| クロノクロック | [chronoclock](chronoclock/README.md) | UTF-8、GBK | CPZ6 | `cmvs-20150405` |
| アマツツミ | [amatsutsumi](amatsutsumi/README.md) | UTF-8、GBK | CPZ6 | `cmvs-20160201` |
| リアライブ | [realive](realive/README.md) | UTF-8、GBK | CPZ7 | `cmvs-20190305` |

## 使用约定

从仓库根目录执行命令。需要 Python 3.10+。除 asukimi 的 GBK 转换外，所有转换均需要本项目要求的 Rust/Cargo 和已缓存的依赖；启动桌面版也需要 Rust/Cargo。

输入只读。输出目录必须不存在，且位于原版游戏目录之外。转换器复制原版资源、程序和配置，再写入转换结果；不安装补丁 exe/DLL 或剧情存档。修改的封包保持原版格式，其他封包原样复制。完成后生成 `conversion.json`，记录编码、输入身份和输出校验值。失败留下的目录不能直接使用，重试时另选新目录。

运行时显式指定游戏的 `--profile`、输出编码和包含所需字形的字体，使用独立的 `--user-data-dir` 保存设置及进度。转换器保留的原版 exe 供身份识别，游戏通过 cmvs-rs 启动。

## 选择编码

上述游戏的转换命令均支持：

| 转换参数 | 行为 | 启动参数 |
| --- | --- | --- |
| `--encoding utf-8` | 默认值，保留源文本字符；也接受 `utf8` | `--encoding utf-8` |
| `--encoding gbk` | 使用旧版 GBK 输出规则 | `--encoding gbk` |

GBK 字符转换规则（各游戏按需使用）：

| 原字符 | 转换后 |
| --- | --- |
| ♪ | ☆ |
| ・ | · |
| 䇲 | 筴 |

リアライブ的额外颜文字替换见[游戏说明](realive/README.md)。

两套输出请使用不同的新目录和独立用户目录。切换编码须从原始游戏和补丁重新转换；旧输出中已被替换的字符不能靠再次转码恢复。

asukimi 的源补丁为 GBK。UTF-8 模式逐字符转码，并重定位字符串内部引用；无法恢复源补丁中已经丢失的字符。转换器依据原版指令布局，仅修复已核对的两处补丁误覆盖（`snas09.ps2`、`snma10.ps2`），在 `conversion.json` 中记录；其他指令变化会报错。GBK 模式保持原有输出。

## 验证范围

编码选择、字符串引用重定位和资源封包有自动测试。各游戏此前的实际运行记录来自 GBK 输出；UTF-8 的验证情况见各游戏说明。完整路线及原版存档互操作尚未验证；各补丁的已知问题见游戏说明。

封包格式迁移已覆盖 miranosu、hapymaher、chronoclock 1.0/1.05、amatsutsumi 的 UTF-8/GBK 共 10 种组合，与此前 CPZ3 输出逐项解包对照一致；既有运行记录来自迁移前。
