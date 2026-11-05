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

## 运行

```powershell
cargo run --offline --release --features desktop,audio-output,wgpu-renderer --bin cmvs-desktop -- "<游戏目录>" --encoding shift-jis --user-data-dir local/hapymaherfd_re/play-user --font "<日文字体文件>" --audio native --renderer wgpu
```

使用独立用户目录保存进度。正常存档不需要 `--continue-without-snapshot`；Windows 可使用系统 MS Gothic 字体。完整分支、鉴赏页面、特殊保存点和其他平台仍需独立验证。
