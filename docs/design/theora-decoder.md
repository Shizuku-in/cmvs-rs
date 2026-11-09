# 自研 Theora 解码器

2026-11-08，`cmvs-video` 用包内的 Rust 解码器替换运行时 `oxideav-theora`，随后也移除了该开发依赖。`TheoraFactory`、`PrefetchTheoraFactory`、`TheoraStream` 以及宿主接入方法保持不变。Ogg 分包仍由 `ogg` 处理；MGV 容器和名义帧映射仍由 core 处理。旧解码器的验证结果保留为固定回归数据，当前依赖只有 `cmvs-core` 和 `ogg`。

## 支持范围

接受 Theora 3.2、YUV420；版本检查与旧后端一致，revision 允许前向兼容。实现 identification/comment/setup 三个头部、80 个传输 Huffman 表、量化范围插值、全部八种宏块模式、前帧与 golden 参考、四运动向量及色度平均、最多三个块量化器、DC 预测、反量化、整数 IDCT 和环路滤波。空数据包复用前帧，流首必须是非空关键帧。

保留 picture/coded 两种 MGV 尺寸、bottom-up 平面、奇数裁剪左边缘的色度取样、BT.601 RGBA 转换、重复帧、倒退重放及工作线程预解码。损坏包报错；失败后的内部解码器不继续提交参考帧，外层 `TheoraStream` 从头重建以维持既有重试行为。

4:2:2、4:4:4 和旧 VP3 兼容版本明确拒绝，与原 MGV 接口的格式限制一致。合成差分通过不代替各游戏或真机验证。

## 实现与性能取舍

源码位于 [`src/theora`](../../crates/cmvs-video/src/theora/)：

| 文件 | 职责 |
|---|---|
| `bits.rs` | 有界 MSB 位读取，64 位缓存按 32 位补充，8 位 Huffman 前缀表；尾部 lookahead 补零不增加可消费位数 |
| `headers.rs` | 三个头部的校验和 setup 表；六组 × 64 档量化矩阵在初始化时生成 |
| `geometry.rs` | superblock/Hilbert 编码顺序、宏块和三个平面的栅格映射 |
| `packet.rs` | 复用编码块列表和系数缓冲；RLE、系数 token 查表展开、DC 预测；解码时同步压缩活动列表 |
| `dsp.rs` | 标量整数 IDCT、完整/稀疏 SSE2 IDCT、整块 SIMD 运动补偿及残差叠加 |
| `mod.rs` | 运动补偿、重建、环路滤波、三个 YUV 缓冲的参考轮换 |

量化矩阵在 setup 时生成，头部验证后才分配画面和几何缓冲。逐包解码复用临时向量和 YUV 平面，三个缓冲按 previous/golden/current 角色轮换。含未编码块的 inter 帧先整面复制前帧，再只重建编码块；全编码帧直接写完整表面。只清零当前编码块的系数，系数解码与活动列表压缩合为一次遍历。Ogg 分包、RGBA 输出与外层最近帧缓存仍有分配/复制。

`cmvs-video` 单独采用 `unsafe_code = deny`、`unsafe_op_in_unsafe_fn = deny`；workspace 默认 forbid 保持不变。仅 `dsp.rs` 的私有内核局部允许 unsafe，入口使用固定数组或已检查完整跨度的 8×8 切片证明读写范围，每个 unsafe 块说明 CPU 指令与内存前提。

| 架构 | IDCT | 预测值与残差相加 |
|---|---|---|
| x86-64 | SSE2，含最多 3/10 个系数的稀疏路径 | SSE2 整块运动补偿、向下取整平均和饱和叠加 |
| AArch64 | 标量，含最多 3/10 个系数的稀疏路径 | NEON 行平均、饱和加法与窄化 |
| ARMv7 / PS Vita 及其他架构 | 标量，含稀疏路径 | 标量 |

SSE2 使用 x86-64 基线能力，无 AVX 要求；支持的 AArch64 目标启用 NEON。SIMD 保留 Theora 指定的有符号高位乘法、16 位截断和舍入语义。最终 IDCT 的 `(X + 8) >> 4` 不能直接用溢出的 16 位加法实现；SSE2 用算术右移加舍入位完成。残差叠加先饱和到有符号 16 位，再饱和为 u8，与标量宽化加法后 clamp 一致。

## 验证

默认 `cargo test --offline -p cmvs-video` 覆盖：

- 原 32×32 MGV 的 RGBA 哈希、重复/倒退访问、picture/coded 裁剪、奇数左边缘及线程预解码。
- 四段 libtheora 合成流共 72 帧；非完整 superblock、宽高不对称、可见区域裁剪、高低质量及噪声。检查完整编码平面的固定哈希和 FFmpeg 可见区域哈希。
- 两段额外固定流共 16 帧，覆盖 golden 参考、四运动向量、四个量化范围和三个块量化器；六组全部 64 档量化矩阵各自检查固定哈希。全部六段流的完整编码平面均已独立通过 FFmpeg 复核。
- MSB 位读取的所有字节内偏移与 0～32 位读取长度、单叶与长 Huffman 码、4129 长 run 的新 BIT 规则、截断/篡改包、非法头部和尺寸预算。
- 2,048 组稀疏/全范围 i16 系数及 9,216 组稀疏 IDCT 的标量/SSE2 对照，同时检查旧实现输出的固定哈希；全部系数 token 的 extra bits 组合也检查固定哈希。另覆盖 signed overflow 的标量/SIMD 像素叠加对照。

默认测试不启动 FFmpeg，fixture 和固定哈希已入库；来源、序列化方式与复核命令见[测试数据说明](../../crates/cmvs-video/tests/fixtures/README.md)。`generate_theora.py --verify` 可用 FFmpeg 复核全部六段流，无需旧解码器。

移除开发依赖后，全 workspace/all-features 1,366 项测试通过、38 项默认忽略，严格 Clippy、格式与 AArch64 Android 测试编译检查通过。另显式通过五项外部检查：裁剪流的可见/完整编码区域 raw YUV 对照、真实 `door.mgv` 的完整编码区域 FFmpeg 对照，以及合成/真实 MGV 的播放映射与重播；未重跑全部真实影片。Cargo 配置、锁文件和包含开发依赖的完整依赖树均不再包含 oxideav。

实现替换时通过 Linux 全 workspace、全 feature 测试（当时 1,359 项通过、36 项默认忽略）、全 workspace/all-targets/all-features 严格 Clippy、格式检查、AArch64 Android 库及测试编译检查。后续新增的真实 MGV 播放用例同样默认忽略。Switch/Vita 的 Linux 构建仅为开发机替身。AArch64 内核只编译检查，尚无运行对照；当前 Docker 服务不可用，未执行 Switch/Vita 交叉构建。真实游戏素材的后续验证见下节；未重跑原 DLL，既有原 DLL 验证范围不能直接归给新后端。

## 真实游戏影片

用户提供的《ハピメア-Fragmentation Dream- REGRET END》目录含七段独立 MGV1，合计 21,515 个名义帧、20,089 个 Theora 解码包。全部编码表面为 1920×1088，picture 为 1920×1080、24 fps。全部解码包的完整 Y/Cb/Cr 与 `oxideav-theora` 0.0.12 逐字节一致，全部可见区域与 FFmpeg 8.0.1 的 top-down yuv420p 逐字节一致。原始影片只读，提取的 Ogg 与日志保留在忽略目录，未将真实素材提交入库。

初始版本在相同 CPU、release、单线程下，按解码包数量加权，自研 **3.384 ms/帧**、旧后端 **9.049 ms/帧**，**2.67 倍**；单片 2.14～3.22 倍。这是包解码时间，排除 Ogg I/O、FFmpeg 管道、比较、色彩转换、哈希与呈现。逐片结果、输入 SHA-256、抽样输出哈希及 MGV 播放验证范围见[游戏档案](../games/hapymaherfd_re/README.md#mgvtheora-自研后端验证2026-11-08)；后续优化的独立计时见下节。

大影片可以用 `CMVS_THEORA_FFMPEG=/usr/bin/ffmpeg` 替代 `CMVS_THEORA_YUV`；忽略测试启动 FFmpeg 并逐帧读取 stdout，只保留一帧参考数据，不保存整部原始视频。比较失败时会回收子进程，并报告第一个不同的平面/帧/样本位置。

初始版本的 MGV 完整播放验证使用真实文件的容器索引与公共 `TheoraStream`：七段影片的全部 21,515 个名义帧与旧后端转换得到的完整 1920×1088 RGBA 逐字节一致，包含 1,426 次重复映射；每段均通过 EOF 及结束后回退至帧零。当时默认视频测试 16 项通过、两项外部素材用例忽略；对七段影片分别显式执行这两项用例，共 14 次通过。严格 Clippy、格式和 AArch64 Android 测试编译检查通过，原游戏内容哈希复核未改变。

```sh
CMVS_THEORA_MGV="<绝对路径>/data/video/op.mgv" \
  cargo test --offline --release -p cmvs-video external_mgv_playback_matches_packet_decoder -- --ignored --nocapture
CMVS_THEORA_OGV="<提取的绝对路径>/op.ogv" CMVS_THEORA_FFMPEG=/usr/bin/ffmpeg \
  cargo test --offline --release -p cmvs-video external_corpus_matches_ffmpeg -- --ignored --nocapture
```

上述为当前命令：MGV 用例对照直接包解码与公共播放接口，检查容器映射、重复帧、RGBA、EOF 与回退重播，不再把同一解码器当作独立像素参考。像素独立对照由 FFmpeg 用例承担；加 `CMVS_THEORA_CODED=1` 可关闭 FFmpeg 裁剪并比较完整编码表面，原始 YUV 文件也应使用相应尺寸。上文旧解码器的逐字节结果属于已完成的历史验证。

## 接近 FFmpeg 的优化（2026-11-08）

在同一 Ryzen 9 9900X、单线程、release、纯 YUV 条件下，七段真实影片各完整运行两次，按 20,089 个解码包加权，优化后 **1.389 ms/帧**，FFmpeg 8.0.1 原生 Theora 解码器 **1.322 ms/帧**，平均差距 **5.1%**。单片最大差距约 12.3%；`edc`、`edd` 更快。这一结果限于当前素材与桌面 CPU，不推断其他码流或 Switch/Vita 真机表现。

| 影片 | 优化前 ms/帧 | 优化后 ms/帧 | FFmpeg ms/帧 |
|---|---:|---:|---:|
| `door` | 2.822 | 1.220 | 1.282 |
| `eda` | 4.059 | 2.293 | 2.042 |
| `edb` | 2.932 | 1.315 | 1.246 |
| `edc` | 1.948 | 0.551 | 0.639 |
| `edd` | 1.918 | 0.523 | 0.619 |
| `op` | 3.498 | 1.837 | 1.683 |
| `opre` | 3.942 | 2.264 | 2.077 |
| 加权平均 | 2.968 | 1.389 | 1.322 |

主要变化为位缓存、系数 token 预展开、编码块列表、整块 SIMD 重建，以及稀疏 IDCT。整块重建在进入 SIMD 前验证跨度，内部省去逐行切片检查；半像素平均严格向下取整，DC 与 AC 使用各自内核。边缘运动向量继续按像素复制边界。这批影片的滤波限制值为零，本轮未修改环路滤波。

优化后的全部 20,089 个包再次与 `oxideav-theora` 比较完整编码平面，与 FFmpeg 比较可见区域，均逐字节一致。新增测试覆盖所有系数 token 的 extra bits 组合、跨字缓存尾部、全部像素平均组合、稀疏 IDCT 的有符号截断和舍入，以及未对齐行、保护字节、短切片和跨度溢出。全 workspace 1,366 项测试、严格 Clippy、格式和 AArch64 Android 测试编译检查通过。

性能计时与像素比较分别执行，避免另一套解码器和 FFmpeg 管道竞争缓存。优化后使用包内的 `external_decoder_timing`，逐帧只计 `Decoder::decode_frame`，输出经 `black_box` 保留；优化前为相同源码入口的独立测量。FFmpeg 对 `avcodec_send_packet` / `avcodec_receive_frame` 计时，强制原生 `theora` 解码器和一个 codec 线程。双方均排除头部初始化、Ogg I/O、RGBA 转换、哈希、比较、呈现及进程启动。不要把这些独立计时与上节同时运行 oracle 的耗时交叉计算加速比。

```sh
CMVS_THEORA_OGV="<提取的绝对路径>/op.ogv" \
  cargo test --offline --release -p cmvs-video --lib external_decoder_timing -- --ignored --nocapture
```

详细数据和计时脚本位于 `local/hapymaherfd_re/theora-2026-11-08/optimization-2026-11-08/` 的 `performance.json`、`validation.json` 及逐片日志；优化前的独立基准在同级 `ffmpeg-speed/`。真实素材和本地输出不入库。

## 重复帧与复杂画面的播放优化

预取只领先当前请求的映射包一个包，避免重复名义帧挤掉当前 RGBA、触发从片头重解。播放器复用重复映射的画面身份，减少解码请求和纹理上传。短 Huffman 码与附加位合并读取；非 x86 IDCT 根据有效系数使用 2×2/4×4 稀疏路径，保留整数截断和舍入。

回归检查顺序播放不回退、显式倒退仍重播、重复映射保留画面身份、Huffman 包尾边界，以及稀疏/完整 IDCT 的逐值一致性。真实 MGV 可另运行下列用例，对比全部名义帧并统计回退次数和请求耗时；它同时运行同步解码器作对照，耗时不代表设备帧率。

```sh
CMVS_THEORA_MGV="<绝对路径>/data/video/op.mgv" \
  cargo test --release -p cmvs-video --test video external_prefetch_playback_never_rewinds_forward_requests -- --ignored --nocapture
```

## Switch 复杂画面与预解码

工作线程默认保留最多 16 张 RGBA / 64 MiB，缓存命中直接返回，不等待正在解码的后续包。Switch 用 `with_lookahead` 改存 YUV420，缓冲上限 192 MiB / 144 帧，仅显示时转换 RGBA；大帧至少保留两帧，当前 RGBA 和解码器参考平面另计。通知通道有界，仅保留最新播放目标；停止、倒退、错误与 EOF 校验仍通过同步请求。

Huffman 初始化时展开 10 位以内的码与附加位，复杂/长码保留原路径；固定包尾测试对照逐树解码，保证 lookahead 不消费补零。模拟器验证使用独立的逐包及按时钟推进基准，分别统计解码耗时和播放请求阻塞；不把桌面计时当作 Switch 帧率。

Ryujinx 1.3.3 的 REALIVE OP 无呈现播放基准（1280×720、24 fps）：一帧预取跳过 188 个名义帧、26 次等待超过 100 ms，最大 1726 ms；YUV 长队列未跳帧、无超过 100 ms 的等待，最大 11.2 ms。该基准按时钟请求整段影片，不含 VM 与 GPU 呈现，不代表完整游戏帧率。

## 初始版本的 1080p 合成基准

Ryzen 9 9900X，Linux x86-64，release，单线程；FFmpeg 8.0.1、libtheora 编码，`testsrc2` 1920×1080、24 fps、120 帧、quality 5、GOP 48，编码表面为 1920×1088。比较每个数据包的 YUV 解码时间，排除头部初始化、Ogg 读取、RGBA 转换、哈希和呈现：本轮验证中自研约 **2.7～2.9 ms/帧**，旧后端约 **5.5～5.8 ms/帧**，约 **2 倍**；最后一次为 2.872 / 5.503 ms，1.92 倍。该合成流全部编码平面与旧后端一致，可见区域与 FFmpeg rawvideo 逐字节一致。

这是合成流微基准，不与 README 早先 600 帧真实影片的 9.9 ms 直接计算加速比，不推断游戏机帧率。

以下当前命令复核合成流的像素；独立性能计时使用上节 `external_decoder_timing`，旧后端耗时须在历史版本复现：

```sh
theora_tmp=$(mktemp -d /tmp/cmvs-theora.XXXXXX)
ffmpeg -v error -f lavfi -i 'testsrc2=size=1920x1080:rate=24' \
  -frames:v 120 -c:v libtheora -q:v 5 -g 48 -pix_fmt yuv420p -an "$theora_tmp/1080p.ogv"
ffmpeg -v error -threads 1 -i "$theora_tmp/1080p.ogv" \
  -vsync 0 -pix_fmt yuv420p -f rawvideo "$theora_tmp/1080p.yuv"
CMVS_THEORA_OGV="$theora_tmp/1080p.ogv" CMVS_THEORA_YUV="$theora_tmp/1080p.yuv" \
  cargo test --offline --release -p cmvs-video external_corpus_matches_ffmpeg -- --ignored --nocapture
```

本轮输入 Ogg SHA-256 为 `a152aef24b569b336a5c8140ab271be5286d9167e547ea646438b64399aff98d`，FFmpeg 输出 YUV SHA-256 为 `a9966adfa499714cdf25ca273c9802942b1501a6b3b56d0b16329581fb37808d`。Ogg serial/编码器元数据可能随重新生成变化；测试按实际包和平面比较，不依赖这两个文件哈希。

## 来源

- [Xiph Theora I 规范](https://www.theora.org/doc/Theora.pdf)，§2、§6、§7；[规范 LaTeX](https://github.com/xiph/theora/blob/master/doc/spec/spec.tex) 用于读取 PDF 排版缺失的过程。
- `oxideav-theora` 0.0.12，MIT，曾作为头部、算术、码表和边界行为的实现参考及差分 oracle，现已从 Cargo 配置和锁文件移除。参考其分解方式重新组织了整数 IDCT 等算法，固定测试输出也来自该版本，因此继续保留 Karpelès Lab 的 MIT 声明于 [THIRD_PARTY_NOTICES.txt](../../THIRD_PARTY_NOTICES.txt)。
- FFmpeg 8.0.1 / libtheora 用于仓库外生成合成码流和独立解码，未复制 FFmpeg 源码或将其链接进播放器。
