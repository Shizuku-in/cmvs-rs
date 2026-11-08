# Theora 合成测试数据

全部数据均为合成素材，不含游戏资源；默认测试不运行 FFmpeg 或第三方解码器。

- `theora-{tall,wide,crop,noise}.hex`：libtheora 编码的四段 Ogg，共 72 帧。`generate_theora.py` 可重新生成影片和哈希。
- `theora-quant.hex`：4 个 32×32 intra 帧，四个量化范围 `[1,20,21,21]`，三个块量化器 `[0,21,63]`，块选择 `bi % 3`；保留六组 × 64 档量化矩阵的哈希。
- `theora-motion.hex`：12 个 32×32 帧，移动棋盘纹理覆盖 golden 参考与四运动向量；测试同时断言实际解码模式。
- `*.sha256`：逐帧可见区域、top-down Y/Cb/Cr 的 FFmpeg 哈希。
- `*.coded.sha256`：逐帧完整编码平面、bottom-up Y/Cb/Cr 拼接后的哈希。
- `theora-idct-sparse.sha256`：系数数量 2～10 各 1,024 组输出的哈希；每组输出按行序展开为 little-endian i16。

`quant` / `motion` 的编码包及量化矩阵、IDCT、系数 token 的固定期望值，在移除依赖前由 `oxideav-theora` 0.0.12 导出；生成参数与随机种子保留于提交 `70a608a` 的 `theora/tests.rs`、`packet.rs`。原 MIT 声明保留在仓库的 `THIRD_PARTY_NOTICES.txt`。

全部六段流共 88 帧的完整编码平面已用 FFmpeg 原生 Theora 解码器再次独立核对。FFmpeg 的 `-apply_cropping 0` 保留编码表面；仅在计算 coded 哈希时逐平面翻转行序。

复核已提交的影片及哈希：

```sh
python3 crates/cmvs-video/tests/fixtures/generate_theora.py --verify
```

IDCT 和系数 token 的聚合哈希还分别固定在对应测试内；使用明确的 little-endian 字节序，支持跨架构复验。固定输出用作回归基准，不能代替新码流与 FFmpeg 的独立对照。
