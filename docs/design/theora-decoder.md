# 自研 Theora 解码器

2026-11-08，`cmvs-video` 用包内的 Rust 解码器替换运行时 `oxideav-theora`。`TheoraFactory`、`PrefetchTheoraFactory`、`TheoraStream` 以及宿主接入方法保持不变。Ogg 分包仍由 `ogg` 处理；MGV 容器和名义帧映射仍由 core 处理。`oxideav-theora = 0.0.12` 保留为开发依赖，用于完整 Y/Cb/Cr 差分和 IDCT 算术对照，不进入播放器的普通依赖树。

## 支持范围

接受 Theora 3.2、YUV420；版本检查与旧后端一致，revision 允许前向兼容。实现 identification/comment/setup 三个头部、80 个传输 Huffman 表、量化范围插值、全部八种宏块模式、前帧与 golden 参考、四运动向量及色度平均、最多三个块量化器、DC 预测、反量化、整数 IDCT 和环路滤波。空数据包复用前帧，流首必须是非空关键帧。

保留 picture/coded 两种 MGV 尺寸、bottom-up 平面、奇数裁剪左边缘的色度取样、BT.601 RGBA 转换、重复帧、倒退重放及工作线程预解码。损坏包报错；失败后的内部解码器不继续提交参考帧，外层 `TheoraStream` 从头重建以维持既有重试行为。

4:2:2、4:4:4 和旧 VP3 兼容版本明确拒绝，与原 MGV 接口的格式限制一致。合成差分通过不代替各游戏或真机验证。

## 实现与性能取舍

源码位于 [`src/theora`](../../crates/cmvs-video/src/theora/)：

| 文件 | 职责 |
|---|---|
| `bits.rs` | 有界 MSB 位读取，8 位 Huffman 前缀表，长码恢复树遍历；尾部 lookahead 补零不增加可消费位数 |
| `headers.rs` | 三个头部的校验和 setup 表；六组 × 64 档量化矩阵在初始化时生成 |
| `geometry.rs` | superblock/Hilbert 编码顺序、宏块和三个平面的栅格映射 |
| `packet.rs` | 复用标志、模式、向量、系数及游标缓冲；RLE、系数 token、DC 预测 |
| `dsp.rs` | 标量整数 IDCT、SSE2 IDCT、标量/SIMD 残差叠加 |
| `mod.rs` | 运动补偿、重建、环路滤波、三个 YUV 缓冲的参考轮换 |

旧后端对每个编码块重新生成 DC 量化矩阵，有 AC 时还会生成另一张矩阵；新后端只索引 setup 时生成的表。头部验证后才分配画面和几何缓冲。逐包解码不重新分配临时向量或 YUV 平面；三个缓冲按 previous/golden/current 角色轮换，避免每帧复制参考平面。熵解码在每个系数位置后压缩活动块列表，已结束的块不再扫描。Ogg 分包、RGBA 输出与外层最近帧缓存仍有分配/复制，不把内部解码器的性质扩展为整条播放链零分配。

`cmvs-video` 单独采用 `unsafe_code = deny`、`unsafe_op_in_unsafe_fn = deny`；workspace 默认 forbid 保持不变。仅 `dsp.rs` 的私有内核局部允许 unsafe，入口使用固定数组引用证明读写范围，每个 unsafe 块说明 CPU 指令与内存前提。

| 架构 | IDCT | 预测值与残差相加 |
|---|---|---|
| x86-64 | SSE2，一次并行八个 1D 变换 | SSE2 饱和加法与打包 |
| AArch64 | 标量 | NEON 饱和加法与窄化 |
| ARMv7 / PS Vita 及其他架构 | 标量 | 标量 |

SSE2 使用 x86-64 基线能力，无 AVX 要求；支持的 AArch64 目标启用 NEON。SIMD 保留 Theora 指定的有符号高位乘法、16 位截断和舍入语义。最终 IDCT 的 `(X + 8) >> 4` 不能直接用溢出的 16 位加法实现；SSE2 用算术右移加舍入位完成。残差叠加先饱和到有符号 16 位，再饱和为 u8，与标量宽化加法后 clamp 一致。

## 验证

默认 `cargo test --offline -p cmvs-video` 覆盖：

- 原 32×32 MGV 的 RGBA 哈希、重复/倒退访问、picture/coded 裁剪、奇数左边缘及线程预解码。
- 四段 libtheora 合成流共 72 帧；非完整 superblock、宽高不对称、可见区域裁剪、高低质量及噪声。完整编码平面与旧解码器逐字节一致，可见区域的 top-down Y/Cb/Cr 拼接 SHA-256 与 FFmpeg 一致。
- 生成流覆盖 golden 参考、四运动向量；另构造四个量化范围和三个块量化器，并对六组全部 64 档量化矩阵逐元素比较。
- MSB 位读取的所有字节内偏移与 0～32 位读取长度、单叶与长 Huffman 码、4129 长 run 的新 BIT 规则、截断/篡改包、非法头部和尺寸预算。
- 2,048 组稀疏/全范围 i16 系数的标量、SSE2 与旧 IDCT 对照；覆盖 signed overflow 的标量/SIMD 像素叠加对照。

默认测试不启动 FFmpeg，fixture 和独立哈希已入库；[`generate_theora.py`](../../crates/cmvs-video/tests/fixtures/generate_theora.py) 记录生成方法。编码器驱动的用例是对旧实现的差分，不计作新增 FFmpeg 独立验证。

实现替换时通过 Linux 全 workspace、全 feature 测试（当时 1,359 项通过、36 项默认忽略）、全 workspace/all-targets/all-features 严格 Clippy、格式检查、AArch64 Android 库及测试编译检查。后续新增的真实 MGV 播放用例同样默认忽略。Switch/Vita 的 Linux 构建仅为开发机替身。AArch64 内核只编译检查，尚无运行对照；当前 Docker 服务不可用，未执行 Switch/Vita 交叉构建。真实游戏素材的后续验证见下节；未重跑原 DLL，既有原 DLL 验证范围不能直接归给新后端。

## 真实游戏影片

用户提供的《ハピメア-Fragmentation Dream- REGRET END》目录含七段独立 MGV1，合计 21,515 个名义帧、20,089 个 Theora 解码包。全部编码表面为 1920×1088，picture 为 1920×1080、24 fps。全部解码包的完整 Y/Cb/Cr 与 `oxideav-theora` 0.0.12 逐字节一致，全部可见区域与 FFmpeg 8.0.1 的 top-down yuv420p 逐字节一致。原始影片只读，提取的 Ogg 与日志保留在忽略目录，未将真实素材提交入库。

相同 CPU、release、单线程，按解码包数量加权，自研 **3.384 ms/帧**、旧后端 **9.049 ms/帧**，**2.67 倍**；单片 2.14～3.22 倍。这是包解码时间，排除 Ogg I/O、FFmpeg 管道、比较、色彩转换、哈希与呈现。逐片结果、输入 SHA-256、抽样输出哈希及 MGV 播放验证范围见[游戏档案](../games/hapymaherfd_re/README.md#mgvtheora-自研后端验证2026-11-08)。

大影片可以用 `CMVS_THEORA_FFMPEG=/usr/bin/ffmpeg` 替代 `CMVS_THEORA_YUV`；忽略测试启动 FFmpeg 并逐帧读取 stdout，只保留一帧参考数据，不保存整部原始视频。比较失败时会回收子进程，并报告第一个不同的平面/帧/样本位置。

MGV 完整播放验证使用真实文件的容器索引与公共 `TheoraStream`：七段影片的全部 21,515 个名义帧与旧后端转换得到的完整 1920×1088 RGBA 逐字节一致，包含 1,426 次重复映射；每段均通过 EOF 及结束后回退至帧零。默认视频测试 16 项通过、两项外部素材用例忽略；本轮对七段影片分别显式执行这两项用例，共 14 次通过。严格 Clippy、格式和 AArch64 Android 测试编译检查通过，原游戏内容哈希复核未改变。

```sh
CMVS_THEORA_MGV="<绝对路径>/data/video/op.mgv" \
  cargo test --offline --release -p cmvs-video external_mgv_playback_matches_reference -- --ignored --nocapture
CMVS_THEORA_OGV="<提取的绝对路径>/op.ogv" CMVS_THEORA_FFMPEG=/usr/bin/ffmpeg \
  cargo test --offline --release -p cmvs-video external_corpus_and_decode_timing -- --ignored --nocapture
```

## 1080p 合成基准

Ryzen 9 9900X，Linux x86-64，release，单线程；FFmpeg 8.0.1、libtheora 编码，`testsrc2` 1920×1080、24 fps、120 帧、quality 5、GOP 48，编码表面为 1920×1088。比较每个数据包的 YUV 解码时间，排除头部初始化、Ogg 读取、RGBA 转换、哈希和呈现：本轮验证中自研约 **2.7～2.9 ms/帧**，旧后端约 **5.5～5.8 ms/帧**，约 **2 倍**；最后一次为 2.872 / 5.503 ms，1.92 倍。该合成流全部编码平面与旧后端一致，可见区域与 FFmpeg rawvideo 逐字节一致。

这是合成流微基准，不与 README 早先 600 帧真实影片的 9.9 ms 直接计算加速比，不推断游戏机帧率。

复现：

```sh
theora_tmp=$(mktemp -d /tmp/cmvs-theora.XXXXXX)
ffmpeg -v error -f lavfi -i 'testsrc2=size=1920x1080:rate=24' \
  -frames:v 120 -c:v libtheora -q:v 5 -g 48 -pix_fmt yuv420p -an "$theora_tmp/1080p.ogv"
ffmpeg -v error -threads 1 -i "$theora_tmp/1080p.ogv" \
  -vsync 0 -pix_fmt yuv420p -f rawvideo "$theora_tmp/1080p.yuv"
CMVS_THEORA_OGV="$theora_tmp/1080p.ogv" CMVS_THEORA_YUV="$theora_tmp/1080p.yuv" \
  cargo test --offline --release -p cmvs-video external_corpus_and_decode_timing -- --ignored --nocapture
```

本轮输入 Ogg SHA-256 为 `a152aef24b569b336a5c8140ab271be5286d9167e547ea646438b64399aff98d`，FFmpeg 输出 YUV SHA-256 为 `a9966adfa499714cdf25ca273c9802942b1501a6b3b56d0b16329581fb37808d`。Ogg serial/编码器元数据可能随重新生成变化；测试按实际包和平面比较，不依赖这两个文件哈希。

## 来源

- [Xiph Theora I 规范](https://www.theora.org/doc/Theora.pdf)，§2、§6、§7；[规范 LaTeX](https://github.com/xiph/theora/blob/master/doc/spec/spec.tex) 用于读取 PDF 排版缺失的过程。
- `oxideav-theora` 0.0.12，MIT，registry 包与校验和固定在 Cargo.lock；作为头部、算术、码表和边界行为的实现参考及差分 oracle。参考其分解方式重新组织了整数 IDCT 等算法，保留 Karpelès Lab 的 MIT 声明于 [THIRD_PARTY_NOTICES.txt](../../THIRD_PARTY_NOTICES.txt)。
- FFmpeg 8.0.1 / libtheora 用于仓库外生成合成码流和独立解码，未复制 FFmpeg 源码或将其链接进播放器。
