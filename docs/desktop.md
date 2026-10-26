# 原生桌面预览

这是 Windows 实测的研究预览，不是完整剧情播放器。winit 负责窗口与事件，softbuffer 呈现已实现的静态场景或用户明确选择的素材。
应用从正常入口处理已验证的启动命令，到未知命令或引擎快照断点暂停；主断点之后仍运行窗口事件循环。
不生成缺失的引擎对象，不把测试背景保存为剧情快照，不自动执行登记回调。

```powershell
cargo run --features desktop --bin cmvs-desktop -- "E:/Download/AMATSUTSUMI" --encoding shift-jis
cargo run --offline --features desktop --bin cmvs-desktop -- "E:/Download/AMATSUTSUMI" --encoding shift-jis --exit-after 30
cargo run --offline --features desktop --bin cmvs-desktop -- "E:/Download/AMATSUTSUMI" --encoding shift-jis --image-archive "E:/Download/AMATSUTSUMI/data/pack/chip.cpz" --preview-image logo.pb3
cargo test --offline
cargo test --offline --features desktop
cargo clippy --offline --features desktop --all-targets -- -D warnings
```

首次需获取可选依赖，此后在已缓存依赖的环境可以 offline 构建。默认 features 为空；`cargo tree --no-default-features` 只有 cmvs-rs。
game-root 明确指定本次研究过的目录布局：data/pack/start.ps3、script.cpz、save 目录和资源根。
不存在的 system.dat 按原版无修改返回；已有损坏存档、编码错误、未知命令不会静默跳过。

窗口标题必须显式指定 shift-jis、gbk 或 utf-8。使用 encoding_rs 的严格转换，不猜测编码，不用替换字符隐藏解码错误。
图标先验证所有 ICO 条目及嵌入图像尺寸相符，限制在目录可表达的 1..256 像素，之后解码最接近 32×32 的条目。
输入文件上限沿用 4 MiB；并不声称模拟 Windows 所有 DPI/图标自动选择规则。
光标用现有 CUR 解码器读取热点/平面；AND=1 且 XOR 非零的反色像素不能转换为普通 RGBA，原生后端明确拒绝。
本作四份 CUR 没有此类反色像素，均可转换并交给 winit 原生光标接口。

研究控制：

- 当前暂停位置由标准输出报告；已移除遮挡画面的底部状态条。
- 暂停后 F1 恢复默认光标，F2～F5 预览登记的四个光标。它们直接测试后端，不改变 VM、系统变量或脚本 PC。
- Esc 或窗口关闭按钮退出。`--exit-after 1..3600` 指定自动退出秒数，仅用于窗口运行检查。
- 标准输出记录实际应用的标题/图标/光标请求和输入事件统计，不写游戏目录。

`--image-archive` 与 `--preview-image` 必须一起提供，basename 目前仅 ASCII。
通过 CPZ6 读取并解码 PB3 type 1，加载成功后在原生窗口中按比例居中显示，最近邻缩放，直通 alpha 合成。
这些是预览显示策略，尚未核对为原版场景的缩放/混合规则；图像的加载和展示不修改 VM。
显式素材预览保留棋盘格背景；未提供图片时呈现场景清屏色与脚本对象。窗口留边为黑色。
不支持的 codec、模糊 basename、图像损坏或错误尺寸会在创建窗口前明确失败。

宿主输入只观察本窗口的物理按键、鼠标移动/按钮/滚轮和焦点事件。
保持按下集合以供后续适配，失焦时清空，最多保留 256 条事件并统计丢弃数量。
这些事件没有伪装成 CMVS 系统结果，也未绑定到游戏输入指令；当前不会推进剧情或触发已登记脚本回调。
窗口帧缓冲上限 16M 像素。启动采用事件循环内的协作执行预算，单条 VM 指令仍可能超过预算，后续需要性能与调度研究。
当前虚拟 tick 固定 0，窗口 wall-clock 和自动退出时钟不代表已完成 CMVS 计时语义。

验证：原生运行 479 步到 main.ps3 的 PC=0x38d1e，13 个回调，已知整数 4097、位变量 262144。
窗口列表实际返回标题「アマツツミ」（加 `--continue-without-snapshot` 后，0x2161 会改为「アマツツミ　- メイン -」）；日志确认创建窗口、提交 32×32 图标并进入暂停后事件循环。
图标 4096 字节 RGBA 与 Pillow 逐字节一致；用户确认 F2～F5 四个光标及 Esc 正常。
人工检查日志记录四个原生光标、键盘、鼠标事件，退出时 VM 步数仍为 479。
早期窗口阶段的截图请求曾被拒绝；本次静态场景阶段已通过 computer-use 成功截图验证，见下节。
macOS/Linux/Android/iOS 尚未构建或真机验证，当前窗口代码不能直接视为这些平台均已支持。

PB3 素材显示验证：只读加载 chip.cpz 内的 logo.pb3（1280×720，type 1/24 位），
用户确认“显示正常，已关闭”。该原生运行仍以 479 步停于主快照断点，Esc 退出，未运行绘图 opcode。

## 脚本驱动的静态场景

```powershell
cargo run --offline --features desktop --bin cmvs-desktop -- "E:/Download/AMATSUTSUMI" --encoding shift-jis --continue-without-snapshot
```

不指定显式素材时，`scene_render` 按 VM 场景状态呈现，采用 1280×720 本作研究视口，再最近邻按比例适配窗口。
支持顶层模式 0、普通混合、单位缩放/零旋转，按脚本裁切、原点、位置、层次及不透明度显示；子对象呈现与其他变换/混合明确报错。
层次降序、同层按槽序；整数 alpha 舍入及窗口缩放未与 D3D9 做逐像素对照。
本机实测不使用 --preview-image 显示完整白底 Purple Software logo，前置窗口截图确认，Escape 正常退出。
日志停于第 944 次 step、logo.ps3 / 0x364ea / 0x2220，14 个回调；并非完整品牌开场播放成功。
默认不加 --continue-without-snapshot 时仍停于第 479 步的快照断点，窗口保持响应；存档镜像未实现。

## 全屏纯色淡入/淡出

同一条 --continue-without-snapshot 命令现会执行 logo 的 2000ms 黑色覆盖层淡入，完成后执行准备声音的启动请求和 3500ms 计时停留，再完成 2000ms 黑色淡出、经 0x20b3 停止效果音通道、0x202f 销毁全部对象，随后切换到 menu.ps3，窗口标题变为「アマツツミ　- タイトルメニュー -」，经 0x22eb/0x22e5 查询跳过与自动送页标志，载入标题图层 title01_chip.pb3 后停在 0x36704 / 0x2210（音频仍只记录，无声）。
VM 指令级错误（例如读取未写入的预留栈字节）与未处理系统命令的暂停不同，桌面会关闭窗口并以错误退出。
帧调度约 16ms，场景按实际经过的毫秒推进；debug 软件绘制耗时可能使帧率低于 60fps，原版全引擎调度尚未复现。
桌面输入仍只观察，活动的 0x2224 非零参数等待未接入；真实 logo 此处使用 0，无需输入跳过。
默认快照断点保持，命令行的 --virtual-tick 也仍不代表整个引擎时钟。

计时器在 VM 让出后恢复时才消费一次帧增量，8ms 协作预算切片不增加计时，未知命令暂停后保持状态。
本阶段 --exit-after 12 的 Windows 实测到达上述新停点并正常退出；淡出完成时覆盖层仍为不透明黑色，属于脚本设置的终态。
0x20b3 阶段 --exit-after 15 实测越过通道停止，日志停于 logo.ps3 / 0x365ba / 0x202f（一次 1287 步），到期自动退出返回 0；本次只核对日志，未另行截图。
0x202f 阶段 --exit-after 15 实测在第 1330 步进入 menu.ps3，第 1331 步遇到当时未支持的 0x0440 bytes=8 后在截止时间前退出，返回 1；只核对日志。
宽栈预留阶段 --exit-after 15 实测显示标题「アマツツミ　- タイトルメニュー -」，第 1226 步停于 0x22eb，窗口保持到截止时间后返回 0；只核对日志。
0x22eb 阶段 --exit-after 15 实测第 1302 步停于 0x22e5，到期返回 0；只核对日志。
0x22e5 阶段 --exit-after 15 实测第 1434 步停于 0x2210，到期返回 0。另以 --exit-after 25 截取窗口客户区（960×540）：显示 title01_chip.pb3 的 (0,0,1280,720) 裁切区域，整体偏暗；
该层带 alpha 叠在黑色清屏色上，其余标题图层尚未创建，未与原版画面对照。
