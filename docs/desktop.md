# 原生桌面预览

当前（2026-10-27）桌面默认只读 `key.cfg` 并向 VM 注入键鼠动作；鼠标开始游戏与 Enter 跳过 OP 已验证，开始游戏停于 `intproc.ps3` 的 `0x0106` 操作数。`--input record` 保留此前无输入模式。活动游戏的 Esc 交给动作绑定，关闭按钮仍可退出。证据和限制见 [input](input.md)。下文逐阶段记录中的“仅观察输入”等描述保留为历史状态。

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

同一条 --continue-without-snapshot 命令现会执行 logo 的 2000ms 黑色覆盖层淡入，完成后执行准备声音的启动请求和 3500ms 计时停留，再完成 2000ms 黑色淡出、经 0x20b3 停止效果音通道、0x202f 销毁全部对象，随后切换到 menu.ps3，窗口标题变为「アマツツミ　- タイトルメニュー -」，经 0x22eb/0x22e5 查询跳过与自动送页标志，载入标题图集 title01_chip.pb3、建立按钮组并显示按钮子对象，记录 BGM 后完成 1500ms 标题淡入并进入按钮空闲循环；无输入约 30 秒后经 BGM 淡出、2000ms 黑色淡出与 BGM 停止，停在 0x2b2ae / 0x230a（OP 影片尚未实现；音频仍只记录，无声）。
随后清理影片槽、按钮组与全部对象，重置动作序列与镜头，切换到 op.ps3（窗口标题「アマツツミ　- オープニング -」），停在 0x36278 / 0x20c0；场景为空，窗口为黑色清屏色，OP 影片尚未实现。
可跳过等待在桌面上同样以“无输入”运行：观察到的鼠标/键盘不注入，按键绑定尚未核对。

## 渲染帧导出

`--save-frame <新文件.png>` 在窗口退出时（--exit-after 截止、Esc 或关闭）按当时场景以脚本坐标 1280×720 渲染并写出 RGBA PNG；文件已存在时拒绝覆盖。
它直接取自软件呈现器，不受窗口层叠、DPI 或缩放影响，用于取代屏幕截图核对画面。显式素材预览模式下导出该素材本身。
实测 --exit-after 4 导出 Purple Software logo，--exit-after 18 导出淡入完成的完整标题画面（与用户截图一致）；Python zlib 独立校验块 CRC、zlib 流与行过滤字节。
按钮交互阶段 --exit-after 45 实测在第 7785 步停于 0x20a2，导出帧仍为标题画面（按钮均为状态 0 图像，无悬停）；桌面步数随帧调度变化。
BGM 停止阶段 --exit-after 45 实测在第 8069 步停于 0x230a，导出帧为标题画面（覆盖层已移除而影片未开始）。
清理阶段 --exit-after 45 实测在第 8507 步停于 0x2068，导出帧全黑（抽样只有一种颜色 0,0,0）。
镜头阶段 --exit-after 50 实测显示 op.ps3 标题，在第 8559 步停于 0x20c0；只核对日志。
影片计时阶段 --exit-after 70 实测进入 op.ps3 后继续运行（共 65298 步、未停于未处理命令），导出帧全为不透明黑色：OP 帧尚未解码，影片音轨只记录。
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
0x22e5 阶段 --exit-after 15 实测第 1434 步停于 0x2210，到期返回 0。
更正：该阶段及之后由脚本自动截取的“窗口客户区”图像不可靠——窗口未被置于最前，CopyFromScreen 截到的是用户终端的背景图（偏暗的角色插图并非本程序输出），相关结论作废。
按钮组阶段 --exit-after 15 实测第 2229 步到达 0x2215 后，重绘时报 `child object rendering is not implemented` 并返回 1；这是当时的明确报错策略。
子对象呈现阶段由用户在 Windows 上运行 --exit-after 30 并截图确认：黑屏、logo 淡入淡出后显示完整标题画面（title01_chip 上方 1280×720 为星空、标题字与灯笼背景），
5 个子对象呈现「はじめから」「システム設定」「つづきから」「ゲーム終了」及灰色「鑑賞モード」；窗口保持到截止时间。标题没有淡入，因 0x2220/0x2224 位于 0x20a4 停点之后。
按钮只显示状态 0 图像，鼠标悬停/点击尚未接入。

## JBPD 影片画面（当前阶段）

以下命令从正常入口完成 logo、标题、无输入 30 秒、OP 画面播放和返回标题：

```powershell
cargo run --offline --release --features desktop --bin cmvs-desktop -- "E:/Download/AMATSUTSUMI" --encoding shift-jis --continue-without-snapshot
```

桌面明确启用 JBPD 解码，以实际毫秒换算目标帧，变化时只读读取目标资源，不重复解码同一帧。
重绘与 `--save-frame` 都读取影片当前帧；自然结束清除影片图像并恢复场景呈现。
解码失败直接报错，保留最后成功的播放器状态，不能继续输出黑屏并假装成功。
本作 1280×720 影片帧按比例适配窗口；使用 release 预览，debug 的安全整数软件解码可能降低帧率。

Windows release `--exit-after 170 --save-frame <新文件.png>` 已从正常入口执行 142530 步：
进入 op.ps3 后完整播放，随后 main.ps3、menu.ps3，导出完整标题画面并返回 0。
这证明本次无输入路径的画面播放和返回，音轨仍只记录、输入仍未注入、存档镜像仍不可用。
另以最新 release `--exit-after 70` 导出 OP 播放中的 1280×720 帧，64862 步后正常退出，已目视确认含实际影片画面。
格式、逐字节对照、支持范围见 [jbpd-format](jbpd-format.md)。

## 可选原生音频（当前阶段）

```powershell
cargo run --offline --release --features desktop,audio-output --bin cmvs-desktop -- "E:/Download/AMATSUTSUMI" --encoding shift-jis --continue-without-snapshot --audio native
```

默认不加 `--audio native` 仍为记录后端。原生模式已接入 Ogg SE、BGM 前奏/循环与淡变、影片音轨，影片选帧跟随音频源位置。
Windows 175 秒有界检查从正常入口完成 OP 音轨、返回标题并正常退出；其他平台未验证。
源消费进度不是 DAC 硬件时间戳，设备延迟未校准，尚未人工听音确认音质；真实输入仍只观察、不注入 VM。
设备或解码错误明确报错。格式、库版本、二进制证据与 PCM 对照见 [audio-output](audio-output.md)。

## 显式消息记录模式（2026-10-27）

`--record-text` 只记录当前支持的单行 0x2152 字形描述与逐字 alpha 计时，不生成字体像素、不绘制剧情文字。默认仍保留该命令的输出断点。该选项可加到现有显式继续命令：

```powershell
cargo run --offline --release --features desktop --bin cmvs-desktop -- "E:/Download/AMATSUTSUMI" --encoding shift-jis --continue-without-snapshot --record-text
```

本轮仅用仓库外无窗口鼠标/键盘重放验证，记录模式到 sn1001.ps3 / 0xf6f2 / 0x2136，未新增原生窗口实测。支持分支、设置约束与默认停点见 vm-semantics.md；字体后端、消息窗口/文字合成仍待实现。

## 字体与首句文字预览（2026-10-27）

显式 `--font <文件> [--font-face <TTC 索引>]` 可替代 --record-text，用相同的受限消息布局实际光栅化字形，并呈现消息窗口；face 默认为 0，只支持 shift-jis。与 --record-text 同用、缺失字形、非等色渐变、非单位缩放均显式失败。--save-frame 导出与窗口相同的字体合成路径。

```powershell
cargo run --offline --release --features desktop --bin cmvs-desktop -- "E:/Download/AMATSUTSUMI" --encoding shift-jis --continue-without-snapshot --font "C:/Windows/Fonts/msgothic.ttc"
```

首句文字及消息窗口已在无窗口真实重放导出帧中核对；16 秒 Windows 窗口检查验证启动、标题与导出，退出码 0，本轮未在窗口中操作开始游戏。字体、基线和边缘使用 OpenType/ab_glyph 策略，与 GDI 有像素差异，支持边界及证据见 [font-output](font-output.md)。

2026-10-27 手动消息等待已接入：原生用户实测确认第一话动画/声音、首句画面及等待小点，点击后小点消失；当前继续停在未实现的 0x22d0，文字尚不推进到下一句。自动送页分支仍明确报错。运行命令及受限设置见 font-output.md 与 vm-semantics.md。

2026-10-27 遮罩转场：软件呈现器现按原版着色器系数绘制 0x222a 的 Put/Cut 转场，桌面显示同一结果。无窗口探针导出的帧已显示事件 CG 按方块遮罩溶解；本轮未在原生窗口中实测转场。

2026-10-27 画面震动：软件呈现器按 0x2225 的偏移平移场景槽对象或窗口/文字图层，桌面显示同一结果；无窗口探针导出帧中 CG 左移 35 px、消息窗口不动。本轮未在原生窗口中实测震动。

## 启动自动读档黑屏与显式标题预览（2026-10-27）

本机 system.dat 的整数 2198=1，使 main.ps3 尝试恢复 save870.dat；默认在 0x395de / 0x22b5 保留断点，尚无场景画面。桌面新增无参数开关 `--skip-auto-resume`：配置 `Startup::set_skip_auto_resume(true)`，只在首次系统存档成功读取后清零内存整数 2198，保持其他持久变量与所有文件不变。只支持已核对的 Amatsutsumi 游戏 ID 0x17800016，其他/未知 ID 显式报错；不替换签名查询结果，不忽略未知指令，完整恢复仍未实现。

推荐当前画面检查命令（静音）：

```powershell
cargo run --offline --release --features desktop --bin cmvs-desktop -- "E:/Download/AMATSUTSUMI" --encoding shift-jis --continue-without-snapshot --skip-auto-resume --audio record --font "C:/Windows/Fonts/msgothic.ttc"
```

默认未启用该选项时，Windows 原生窗口仍在第 808 步暂停，现把暂停原因和启动选项提示显示在窗口标题中；其他未实现命令显示命令号与脚本/PC，快照屏障显示显式继续选项。提示不是 VM 命令或脚本标题修改。

同一 release 生产构建以记录音频模式运行 14 秒，完成 logo 淡入/停留/淡出、标题淡入，9983 步后正常退出，导出 local/skip-auto-resume-title.png（1280×720），目视确认标题完整、没有黑屏。存档目录所有文件运行前后哈希一致。鼠标/Enter 无窗口重放可从标题开始游戏，分别到 sn1001.ps3 / 0x2b656 / 0x2070（481318/481358 步，SP=32）；模式 2 的原生绘制与该后续命令仍未实现，本轮未在原生窗口点击进入剧情。

真实音频模式在当前 192000 Hz/双声道默认设备上两次异常退出，状态 0xc0000008，日志结束在 sys301.ogg 准备与品牌淡入之后，未生成预定标题帧。原因未定位，本轮不宣称音频设备播放成功，使用 `--audio record` 可独立验证画面。未更改音频后端或用户设备设置。
