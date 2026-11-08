# 《ハピメア-Fragmentation Dream- REGRET END》测试档案

## 样本身份

| 字段 | 内容 |
|---|---|
| 游戏标识 | `hapymaherfd_re` |
| 本地样本目录 | `2023-05-26_HAPYMAHERFD_RE` |
| 英文名称 | Hapymaher -Fragmentation Dream- REGRET END |
| 引擎 / 脚本自报版本 | `cmvs64[ver.3.90]` / `ps3[ver.1.00]` |
| 发行版本与补丁 | 未确认；目录日期仅作为样本标签 |
| 平台与架构 | Windows PC；x86 + x64 |
| 文本编码 | 本轮使用 Shift-JIS；剧情画面抽查通过，未普查全量文本 |
| 引擎 profile | `cmvs-20221105`，基准 `cmvs-20160201` |
| 本地封包签名 | CPZ5（12 个包） |

## 文件身份

| 文件 | 架构 / image base 或签名 | SHA-256 |
|---|---|---|
| `cmvs32.exe` | x86 / `0x400000` | `aa2f76985e69ec5fcee2762051a234c111529e8b339d733dee188b3a62888073` |
| `cmvs64.exe` | x64 / `0x140000000` | `dfbabd2abac5055d3ab2144a983b6c441b87b294d19607e9a4829e2ea954e86c` |
| `data/pack/start.ps3` | `PS2A` | `dc336b0d50c848a12f6dbcd4ebee26b7c1516821ecc16625e5b2ba27f5675b85` |
| `data/pack/script.cpz` | `CPZ5` | `fb93c8d5277be1c1cd812214ccc5a9ded679fcdbde1b98ef17a925d18a104959` |

## 构建与运行接入（2026-11-05）

两侧引擎二进制的 SHA-256、PE 时间戳及分派表均与[本篇 REGRET END](../hapymaher_re/README.md)完全一致，复用 `cmvs-20221105` 的已核对构建语义，包括 1920×1080 画布、1024 子槽、cell 文字、带参数封包、MGV、碎片/粒子及 TextV3SceneV4 快照。未创建重复 profile，也未提升未复核命令的证据等级；749 个处理命令仍为 Unreviewed，243 个为空操作，默认研究策略允许未复核命令。

启动脚本 `0x1c..0x26` 压入常量 `0x17600015`，`0x28` 的 `0x201d` 设置游戏标识。游戏记录独立绑定上表启动脚本、脚本包与该标识；不与本篇的 `0x17600013` 混用存档。自报版本来自样本 `info.txt`，不作为补丁版本证明。

## 自动验证

固定种子 0、虚拟帧间隔 16ms、鼠标、MS Gothic 字体及 Shift-JIS；默认捕获剧情快照。样本自带 CSS1 系统档只读复制到隔离用户目录，非独立构造的全新解锁状态。

- S1–S5 通过：124 个脚本、3,164,910 条指令、429 种系统命令；11 个资源包共 27,027 项资源头（PB3 16,155、Ogg 10,857、CMV 8、MGV 7）。结构通过不代表所有素材载荷或语义均正确。
- R1/D1、I1–I5、V1–V4 通过：18,869 步 / 624 帧到达标题，4 个启用按钮；同输入重放一致，剧情开头支持重启恢复、快照字节往返、同会话恢复及备份槽。
- Windows 桌面默认快照、wgpu Vulkan 和原生音频启动 12 秒正常退出，导出 1920×1080 标题；BGM `bgm35.ogg` 为 44.1kHz 双声道。标题和剧情开头实际合成图已抽查；未验证听感或完整原生交互。

原始报告、截图、隔离存档和研究工具保存在忽略目录 `local/hapymaherfd_re/`；原游戏文件只读，真实素材与存档不入库。

## MGV/Theora 自研后端验证（2026-11-08）

原目录 `data/video/` 的七段 MGV1 均使用 1920×1088 编码表面、1920×1080 可见区域、24 fps、YUV420。合计 **21,515 个名义帧、20,089 个解码包**，全部包的完整 Y/Cb/Cr 与旧后端 `oxideav-theora` 0.0.12 逐字节一致；可见区域与 FFmpeg 8.0.1 的 yuv420p 逐字节一致。此处没有新增原 DLL、原 GPU 或真机画面对照。

公共 `TheoraStream` 的全部 **21,515 个名义帧**也与旧后端转换得到的完整 1920×1088 RGBA 逐字节一致，包含 **1,426 次重复映射**。每段影片均通过 EOF 检查和结束后回退重播帧零。此验证读取原始 MGV，覆盖真实容器索引、尺寸选择、逐包解码、RGBA 转换和缓存；未将其扩展为原生窗口、音频或真机验证。新增验证工具不改变运行时解码器，视频默认 16 项测试及严格 Clippy、格式、AArch64 Android 测试编译检查通过，两项真实素材测试默认忽略并已对七段影片各显式运行。

Ryzen 9 9900X、Linux x86-64、Rust 1.99.1、release、单线程，逐片全包解码计时如下。计时排除头部、Ogg 读取、FFmpeg 管道、比较、RGBA 转换、哈希和呈现：

| 影片 | 名义帧 | 解码包 | 自研 ms/帧 | 旧后端 ms/帧 | 加速 |
|---|---:|---:|---:|---:|---:|
| `door.mgv` | 97 | 94 | 3.097 | 9.175 | 2.96× |
| `eda.mgv` | 3022 | 2928 | 4.400 | 12.162 | 2.76× |
| `edb.mgv` | 4867 | 4796 | 3.297 | 8.317 | 2.52× |
| `edc.mgv` | 3022 | 2693 | 2.381 | 5.101 | 2.14× |
| `edd.mgv` | 4867 | 4062 | 2.397 | 5.200 | 2.17× |
| `op.mgv` | 3168 | 3089 | 3.964 | 11.795 | 2.98× |
| `opre.mgv` | 2472 | 2427 | 4.368 | 14.059 | 3.22× |

按解码包数加权为 **3.384 / 9.049 ms，2.67 倍**。

证据保存在 `local/hapymaherfd_re/theora-2026-11-08/`：`inputs.json` 记录 MGV/Ogg 身份与尺寸，`summary.json` 记录各片计时和抽样哈希，`*-validation.log` 记录旧后端与 FFmpeg 的逐帧检查，`mgv-results.json` 与 `*-mgv.log` 记录独立的公共播放路径检查。实际测试命令见[解码器说明](../../design/theora-decoder.md#真实游戏影片)。提取的 Ogg 和全部验证输出仅留在忽略目录；验证后复核七段原始 MGV 的 SHA-256，内容均未改变。

## 运行

```powershell
cargo run --offline --release --features desktop,audio-output,wgpu-renderer --bin cmvs-desktop -- "<游戏目录>" --encoding shift-jis --user-data-dir local/hapymaherfd_re/play-user --font "<日文字体文件>" --audio native --renderer wgpu
```

使用独立用户目录保存进度。正常存档不需要 `--continue-without-snapshot`；Windows 可使用系统 MS Gothic 字体。完整分支、鉴赏页面、特殊保存点和其他平台仍需独立验证。
