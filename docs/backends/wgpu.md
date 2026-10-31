# wgpu 渲染后端

通过 `--features wgpu-renderer` 构建桌面 GPU 后端，并在运行时选择 `--renderer wgpu`。默认 `--renderer cpu` 使用软件合成和 softbuffer；只启用 `desktop` 仍能独立构建 CPU 后端。未编译 GPU feature、没有适配器、设备/资源失败均明确报错，不自动换回 CPU。

```powershell
cargo run --offline --release --features wgpu-renderer --bin cmvs-desktop -- "<游戏目录>" --encoding shift-jis --continue-without-snapshot --font "C:/Windows/Fonts/msgothic.ttc" --renderer wgpu
```

## 合成和资源

`cmvs-wgpu` 依赖 core、wgpu 30.0.1、pollster 0.4.0，不向 core 引入图形依赖或 unsafe。共享 `prepare_scene` 完成所有语义校验、继承、投影与排序，GPU 严格依序执行 Image/Projected/Rotated/Solid/Fade；换图、条带、震动及父子层次已在共享队列展开。遮罩覆盖率查找表、屏幕第二纹理、纹理减法与六种混合分支由着色器/混合状态执行。

合成目标为逻辑尺寸的 RGBA8Unorm，以原有字节颜色空间计算。非旋转图像保留点采样，投影/缩放/旋转显式执行钳位双线性和8位取整；像素中心保留现有软件路径的半像素约定。窗口表面通过非 sRGB 视图呈现，避免合成结果再被隐式 gamma 编码。窗口比例由共享 Viewport 决定，点采样/线性、黑边与预览棋盘单独呈现。

纹理按像素 SHA-256 与尺寸缓存，留存上限128 MiB，LRU淘汰；超大单图仍受设备限制，超过缓存上限的纹理只供当次使用。缓存上限不是总显存或进程峰值保证。相同静态纹理不重复上传；文件名不作为缓存身份。不可变 RgbaImage 首次计算后缓存内容摘要，缓存状态不参与图像相等性；后续帧复用摘要。每帧仍需生成参数和命令，不承诺一定快于 CPU。影片解码与字形光栅化留在 CPU，图像/字形合成、影片缩放和窗口显示在 GPU。

## 截图和生命周期

成功呈现后保留该帧独立 GPU 画布。`DeviceFrame` 只在即时截图请求时读取最近成功呈现的逻辑帧，包括当前路径的字形或影片，不含窗口黑边、光标或素材预览；失败/跳过呈现不替换上一帧。读回按256字节行对齐并等待映射完成。正常帧不执行GPU到CPU读回。

`--save-frame` 读回所选后端最后一次成功呈现的逻辑画布，退出时不重绘VM中间状态。延迟截图 `0x2304` 仍在核心用 CPU 生成，原有文字/影片屏障与时序不变；不能与即时设备截图混用。

零尺寸和遮挡跳过呈现；超时保留旧帧，下次重试。过期表面重新配置，丢失表面由桌面重建，设备丢失及显存/校验错误向宿主报告。首期只检查 Windows；其他平台虽可复用代码，未完成构建和行为验证。

## 验证

```powershell
cargo test --offline -p cmvs-wgpu -- --ignored --nocapture
$env:CMVS_GAME_ROOT='<游戏目录>'
$env:CMVS_FONT_PATH='C:/Windows/Fonts/msgothic.ttc'
cargo test --offline -p cmvs-desktop --features wgpu-renderer --test wgpu -- --ignored --nocapture
```

第一个测试仅用合成数据，需要原生GPU；默认忽略。它核对裁剪、缩放、旋转、混合、遮罩/第二纹理、纯色/fade、非对齐行宽、重复读回、内容更新和缓存复用。独立混合/采样场景最大逐通道误差不超过2；不使用平均误差判断。

第二个测试只从环境变量读取外部游戏/字体，固定种子0和16ms时钟，抽查真实完整场景并输出最大误差、超过2的字节数及时间。提交耗时、等待读回耗时及缓存热后的重复渲染并等待完成耗时分开记录，均非GPU timestamp测量。游戏结果、原生检查和覆盖限制另见对应游戏研究记录；不代表完整路线或原版D3D逐像素一致。


桌面预算耗尽不触发GPU合成，必须等VM实际Yield。该规则与CPU共用，详见[桌面帧边界](desktop.md#vm-时间片与完整帧边界)。Save菜单切换的专项检查可显式执行：

```powershell
$env:CMVS_SAVE_ROOT='<只读存档来源目录，含system.dat和save000.dat>'
cargo test --offline --release -p cmvs-desktop --features wgpu-renderer external_save_menu_renders -- --ignored --nocapture
```

此测试额外需要前述游戏根与字体环境变量；未指定存档来源时使用游戏的save目录。测试只读来源文件，在target下创建独立用户目录；每条VM指令后模拟宿主预算切片，检测创建子对象的中间状态，只在Yield后更新并绘制。它不代表原生物理鼠标点击或音频设备验证。
