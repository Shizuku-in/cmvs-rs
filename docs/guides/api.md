# API 文档

HTML 文档用于浏览接口和示例，rustdoc JSON 用于程序读取类型、签名和注释。默认生成 `cmvs-core` 的公开 API，不包含第三方依赖的文档。

先按接入层次选择入口：

| 用途 | 推荐入口 |
|---|---|
| 嵌入完整播放器 | [`cmvs-host::Player`](../../crates/cmvs-host/README.md)，负责字体、音频和画面合成 |
| 自定义宿主 | `runtime::{Runtime, RuntimeOptions, FrameDriver}` |
| 读取资源 | `cpz::ArchiveIndex`、`ps3::Script`、`image::load` |
| 单步执行与研究工具 | `machine::Machine`；自行路由命令时使用 `dispatch` |

核心库首页进一步区分宿主服务、引擎状态和底层执行接口。`dispatch` 的登记表供元数据查询，处理函数与参数记账由 core 内部维护。按需依赖对应的子包。

## 生成文档

```sh
rustup toolchain install nightly-2026-01-20 --profile minimal
python3 -I tools/api_docs.py
```

依赖已缓存时可加 `--offline`。输出保存在 `target/api/`，该目录已由 Git 忽略。下表路径均相对于此目录：

| 文件 | 用途 |
|---|---|
| `html/doc/cmvs_core/index.html` | 核心 API 浏览入口 |
| `json/doc/cmvs_core.json` | 类型、签名、trait、重导出与文档注释 |
| `manifest.json` | 包列表、文档路径、编译器版本、功能选项和 JSON 格式版本 |

程序先读取 `manifest.json`，再按其中的路径打开各库的 JSON。`root` 指向根模块，`index` 保存条目详情，`paths` 提供条目路径；解析跨库引用还需对应库的文档。rustdoc JSON 的格式随工具链变化，读取前应检查 `format_version`。

源码路径会经过重映射，JSON 中外部库的本机产物路径仅保留文件名。

## 选择库和功能

`-p` 可重复使用；HTML 和 JSON 使用相同的功能配置：

```sh
python3 -I tools/api_docs.py -p cmvs-host -p cmvs-input
python3 -I tools/api_docs.py -p cmvs-desktop --features audio-output
```

也支持 `--all-features`、`--no-default-features` 和 `--toolchain`。桌面和音频功能需要相应的系统依赖，见 [平台总览](../../platforms/README.md)。默认命令生成当前宿主目标的 API，不覆盖其他平台的条件编译分支。

重复生成可能保留旧文件，本次产物以 `manifest.json` 的列表为准。需要清理旧产物时，先移走 `target/api/` 再运行。

## 只生成 HTML

无需 JSON 时可使用稳定工具链：

```sh
cargo doc --locked --no-deps --lib -p cmvs-core -p cmvs-host
```

打开 `target/doc/cmvs_core/index.html` 即可浏览。模块职责和宿主接入背景见[架构](../architecture.md)及各库的 README。

## 注释约定

API 注释先说明用途，再按需补充参数单位、调用顺序、返回值和错误。示例应通过 doctest 检查。原版地址、内存偏移和反汇编证据放在普通源码注释或研究文档中；公开注释保留必要的支持限制和未确认事项。
