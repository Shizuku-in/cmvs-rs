# アオイトリ / Aoitori 测试档案

## 样本身份

| 字段 | 内容 |
|---|---|
| 游戏标识 | `aoitori` |
| 发行版本与补丁 | 目录标识 `2017-11-24_AOITORI`；具体补丁未确认 |
| 平台、编码 | Windows PC；x86 + x64 可执行文件；研究使用 Shift-JIS |
| 构建档案 | `cmvs-20171114`，基准 `cmvs-20160201` |
| x64 / image base | `cmvs64.exe` / `0x140000000` |
| x64 SHA-256 | `3bccda5d7611412c6938c0c9b06b2195914cfb78763a53f2c235bb6479395ee0` |
| x86 / image base | `cmvs32.exe` / `0x400000` |
| x86 SHA-256 | `b1621c5b504a800344316d72dc4477fbf13a00f15f8308470952429f4013003a` |
| `data/pack/start.ps3` SHA-256 | `10afc37c05c970a898ea263b2a45e553813be8753302754349d8b9c1b801f5fe` |
| `data/pack/script.cpz` SHA-256 | `d2a22d23d600a75ecbabef3c28fa9bdcbe69a33aed756487c9c515e021295cc7` |
| 脚本声明游戏 ID | `0x1810a032` |

两个 switch 各992项：734个处理入口、258个空桩。x64比较得到5条 `Inherited`、2条 `Presumed`、727条 `Unreviewed`；没有把研究运行升级为复核证明。x86仅核对身份和switch表，未做独立语义复核。

## 当前验证

2026-11-01，基线 `57a9348` 加本轮比例ruby实现。完整记录见[研究记录](research.md)，地址及差异见[VM语义](vm-semantics.md)。

| 项目 | 结果与范围 |
|---|---|
| 身份及容器 | S1通过；脚本包为CPZ7，已由现有解码器读取 |
| 脚本结构 | S2通过：86个来源、3,229,155条指令、441种系统命令 |
| 默认闸门 | S3失败：脚本中439种命令默认停止；研究放行不改变该结果 |
| 启动、标题 | `--allow-presumed --allow-unreviewed` 下R1通过L0：624帧、22,050步，`menu.ps3` 5个启用按钮 |
| 启动确定性 | D1通过L1；两次轨迹及最终状态相同，文字分派重构后保持一致 |
| 剧情 | 标题按钮0可进入剧情；比例ruby屏障已越过，R2首条路线18,129,135步、8,624帧停在 `sn102.ps3 / 0xa298 / 0x22a5` 未实现；未到选择或结局 |
| 页面与设置 | P1/C1在来源矩形越界处失败；标题设置入口另遇到 `0x22ed` 未实现，不能宣称设置验证通过 |
| 原版存档 | S5跳过：游戏save目录没有system或剧情存档 |
| 剧情快照、存读档 | 无已核对布局；研究探针继续但不构建剧情快照，不支持存读档 |
| 画面、原生窗口、音轨、其他平台 | 本轮未验证；字体使用OpenType度量，不保证GDI像素一致 |

## 重现

```powershell
cargo run --offline --release -p cmvs-probe -- "E:/Download/Purple Software/2017-11-24_AOITORI" --out "local/aoitori/new-run" --checks S1,S2,S3,S5,R1,D1,R2 --font "C:/Windows/Fonts/msgothic.ttc" --allow-presumed --allow-unreviewed --max-steps 100000000
```

输出目录必须不存在。固定种子0、16ms虚拟帧、鼠标、新用户目录；原游戏只读。包含S3和R2时预期非零退出，不能把其余通过项视为完整游戏兼容。

已知命令的共享分派迁移已收口，完整架构验收仍待完成，见[分派设计](../../design/dispatch.md)第25节。本轮继续Aoitori研究语义：实现比例ruby后，R1/D1结果及哈希保持不变，剧情推进到新的 `0x22a5` 屏障；后续优先分析该命令及其对象/资源行为。
