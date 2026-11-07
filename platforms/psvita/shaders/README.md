# Vita GPU 着色器

`scene_v.cg` / `scene_f.cg` 移植自 `crates/cmvs-wgpu/src/scene.wgsl`，参数布局见 `cmvs_core::scene_render::shader`，纹素寻址、双线性权重与舍入按 CPU 合成器（`scene_render.rs` 的 `bilinear_address`）。同目录的 `.gxp` 是编译结果，由 `runtime/shaders.S` 嵌入播放器，构建播放器不需要编译器。

GXM 只接受离线编译的 GXP，开源工具链没有 Cg 编译器。唯一可用的是 Sony 的运行时编译器 `libshacccg.suprx`（PSM Runtime 内，用 ShaRKF00D / ShaRKBR33D 提取解密），不可再分发，仓库不包含。修改 `.cg` 后重新生成：

1. 把解密的 `libshacccg.suprx` 放到 Vita3K 数据目录的 `ur0/data/`（或真机 `ur0:data/`）。
2. 在构建镜像中生成编译工具 `cmvs-vita-shaderc.vpk`（`CMVS00098`，源码 `../shaderc/main.c`，链接 vitaShaRK）：

   ```sh
   docker run --rm -v "$PWD:/src" -w /src cmvs-vita-build sh platforms/psvita/scripts/build-shaderc.sh
   ```

3. 在宿主机运行 `sh platforms/psvita/scripts/compile-shaders.sh`：安装工具与 `.cg` 到 Vita3K，运行、等待完成、关闭 Vita3K，成功后把 `.gxp` 复制回本目录。`VITA3K`（可执行文件，缺省 `~/Applications/Vita3K/Vita3K.AppImage`）与 `VITA3K_DATA`（数据目录，缺省 `~/.local/share/Vita3K/Vita3K`）可覆盖。诊断写在 `ux0:data/cmvs-shaderc/log.txt`。

在真机上：安装工具 VPK，把 `.cg` 放进 `ux0:data/cmvs-shaderc/`，运行后取回 `.gxp`。

编译工具静态链接 vitaShaRK（LGPL-3.0），只在本地生成着色器，不随播放器发布。

工具以 O2、关闭 fastmath / fastprecision / fastint 编译，保证浮点运算与软件合成一致。Cg 中 `row`、`column` 等是保留字。
