# wgpu 渲染后端

通过 `--features wgpu-renderer` 构建桌面 GPU 后端，并在运行时选择 `--renderer wgpu`。默认 `--renderer cpu` 使用软件合成和 softbuffer；只启用 `desktop` 仍能独立构建 CPU 后端。未编译 GPU feature、没有适配器、设备/资源失败均明确报错，不自动换回 CPU。

```bash
cargo run --offline --release --features wgpu-renderer --bin cmvs-desktop -- "<游戏目录>" --encoding shift-jis --continue-without-snapshot --font "C:/Windows/Fonts/msgothic.ttc" --renderer wgpu
```

## 合成和资源

`cmvs-wgpu` 依赖 core、wgpu 30.0.1、pollster 1.0.1，不向 core 引入图形依赖或 unsafe。共享 `prepare_scene` 完成所有语义校验、继承、投影与排序，GPU 严格依序执行 Image/Projected/Rotated/Solid/Fade，每条命令的着色器参数由 core 的 `scene_render::shader` 生成（Switch deko3d 后端共用同一布局）；换图、条带、震动及父子层次已在共享队列展开。遮罩覆盖率查找表、屏幕第二纹理、纹理减法与六种混合分支由着色器/混合状态执行。

合成目标为逻辑尺寸的 RGBA8Unorm，以原有字节颜色空间计算。非旋转图像保留点采样，投影/缩放/旋转显式执行钳位双线性和8位取整；像素中心保留现有软件路径的半像素约定。窗口表面通过非 sRGB 视图呈现，避免合成结果再被隐式 gamma 编码。窗口比例由共享 Viewport 决定，点采样/线性、黑边与预览棋盘单独呈现。

纹理按像素 SHA-256 与尺寸缓存，留存上限128 MiB，LRU淘汰；超大单图仍受设备限制，超过缓存上限的纹理只供当次使用。缓存上限不是总显存或进程峰值保证。相同静态纹理不重复上传；文件名不作为缓存身份。不可变 RgbaImage 首次计算后缓存内容摘要，缓存状态不参与图像相等性；后续帧复用摘要。每帧仍需生成参数和命令，不承诺一定快于 CPU。影片解码与字形光栅化留在 CPU，图像/字形合成、影片缩放和窗口显示在 GPU。

RgbaImage 的克隆共享不可变像素及懒计算摘要，临时字形对象计算的摘要也供字体缓存原件复用。场景参数使用按需扩容的统一 Uniform Buffer，每帧一次上传，通过满足设备对齐要求的动态偏移选择各 draw；绑定范围仍保留完整的 280-float 布局。TextureView 随纹理缓存，BindGroup 按三个视图的组合复用，仅保留本帧使用且纹理仍在缓存内的组合，缓冲扩容时重建。提交顺序保证下一帧的参数上传不会覆盖尚未执行的上一帧绘制，无需每帧等待 GPU 空闲；逻辑画布仍各帧独立。

窗口及影片缩放在整数对齐、源尺寸等于视口尺寸时只读取一个纹素，即使启用了线性模式；透明源仍执行预乘 alpha，黑边与预览棋盘保持原规则。实际缩放继续使用手工四点采样和原有取整。Switch GLSL 同步使用此 1:1 快路径。

## 截图和生命周期

成功呈现后保留该帧独立 GPU 画布。`DeviceFrame` 只在即时截图请求时读取最近成功呈现的逻辑帧，包括当前路径的字形或影片，不含窗口黑边、光标或素材预览；失败/跳过呈现不替换上一帧。读回按256字节行对齐并等待映射完成。正常帧不执行GPU到CPU读回。

`--save-frame` 读回所选后端最后一次成功呈现的逻辑画布，退出时不重绘VM中间状态。延迟截图 `0x2304` 仍在核心用 CPU 生成，原有文字/影片屏障与时序不变；不能与即时设备截图混用。

零尺寸和遮挡跳过呈现；超时保留旧帧，下次重试。过期表面重新配置，丢失表面由桌面重建，设备丢失及显存/校验错误向宿主报告。首期只检查 Windows；其他平台虽可复用代码，未完成构建和行为验证。

## 验证

浏览器宿主使用同一合成器，通过异步初始化、Worker OffscreenCanvas 呈现和按需异步截图读回适配 WebGPU。原生 `Renderer::new` 与同步读回保持原接口；浏览器使用 `Renderer::new_async` 和 `GpuFrame::read_async`。浏览器构建及回归入口见 [Web 后端](../../platforms/web/README.md)。

```bash
cargo test --offline -p cmvs-wgpu -- --ignored --nocapture
$env:CMVS_GAME_ROOT='<游戏目录>'
$env:CMVS_FONT_PATH='C:/Windows/Fonts/msgothic.ttc'
cargo test --offline -p cmvs-desktop --features wgpu-renderer --test wgpu -- --ignored --nocapture
```

第一条命令运行合成测试，需要原生GPU；默认忽略。它们核对裁剪、缩放、旋转、混合、遮罩/第二纹理、纯色/fade、非对齐行宽、重复读回、内容更新和缓存复用；也覆盖参数缓冲扩容与复用、非零偏移的遮罩 LUT、多帧连续提交后再读回旧帧，以及带透明像素和黑边的 1:1 呈现。独立混合/采样场景最大逐通道误差不超过2；不使用平均误差判断。

2026-11-06 第一轮复用优化的局部实测：Linux / RTX 5070 Ti / Vulkan / 驱动595.99.02 / Release，基线 `fc710f8`。1280×720画布上绘制512个32×32、共享同一纹理的图块，20次预热后统计80次中位数：资源准备及提交2.296→0.195 ms，渲染并等待完成4.118→0.228 ms。另对同一36×36图像连续512次克隆并取缓存键，每组重复80次，中位数6.491→0.00574 ms；修改后首个副本初始化摘要，后续副本共享。均为宿主时钟测量，不含窗口呈现，也不代表不同纹理组合、真实游戏FPS或Switch性能。1:1呈现快路径本机仅见微小耗时差，不据此宣称独立加速比。两项原生GPU回归通过，Switch GLSL通过uam编译；本轮未做Switch真机验证。

第二个测试只从环境变量读取外部游戏/字体，固定种子0和16ms时钟，抽查真实完整场景并输出最大误差、超过2的字节数及时间。提交耗时、等待读回耗时及缓存热后的重复渲染并等待完成耗时分开记录，均非GPU timestamp测量。游戏结果、原生检查和覆盖限制另见对应游戏研究记录；不代表完整路线或原版D3D逐像素一致。


桌面预算耗尽不触发GPU合成，必须等VM实际Yield。该规则与CPU共用，详见[桌面帧边界](desktop.md#vm-时间片与完整帧边界)。Save菜单切换的专项检查可显式执行：

```bash
$env:CMVS_SAVE_ROOT='<只读存档来源目录，含system.dat和save000.dat>'
cargo test --offline --release -p cmvs-desktop --features wgpu-renderer external_save_menu_renders -- --ignored --nocapture
```

此测试额外需要前述游戏根与字体环境变量；未指定存档来源时使用游戏的save目录。测试只读来源文件，在target下创建独立用户目录；每条VM指令后模拟宿主预算切片，检测创建子对象的中间状态，只在Yield后更新并绘制。它不代表原生物理鼠标点击或音频设备验证。
