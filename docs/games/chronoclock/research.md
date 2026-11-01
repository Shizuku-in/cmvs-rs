# CHRONOCLOCK 研究记录

## 2026-11-01：全新用户目录、快速存读档与回想跳转

原生运行前确认 `local/chronoclock/native-user-fresh-01` 不存在，不预置解锁CSS1或剧情槽；允许宿主正常使用原游戏只读模板，因此这不验证“游戏本身也缺少CSS1模板”的另一路径。使用运行指南同样的研究参数、软件绘制和原生音频，首次显示只有START/LOAD/SYSTEM/EXIT的初始标题。START进入「プロローグ１」，在「７月――夏。」通过工具栏保存到空槽001，产生141339字节的 `save000.dat`。正常关闭创建用户 `cmvs.cfg` 与 `system.dat`，退出码0；随后同目录重启，LOAD能看到新槽缩略图并恢复同一句。

重启会话继续验证工具栏クイックセーブ：确认后产生 `save860.dat`；推进到「その日、学園の一番空に近い場所で、俺は失恋を経験しようとしていた。」后，クイックロード确认恢复「７月――夏。」。再次推进，滚轮上打开回想，点击旧句弹出クイックジャンプ确认，选择是后实际回到旧句剧情等待。没有用普通读档代替回想跳转，也没有注入探针场景推进。最后正常关闭，退出码0、输入丢弃0，窗口清单已为空。

仅实际操作验证及文档，无新增测试或运行时修改。忽略目录证据：`native-window-05.log` SHA-256 `7bdb043843fcb1ec5cc0f6583f16ff7d6e31901da66dd4d7ceedbee01953fc92`；重启/快速操作/回想记录 `native-window-06.log` 为 `99386182de13aa5412e4e5fdc8021e7ddf0d8b4904694acb72b257b2ebd2c849`；普通槽 `save000.dat` 为 `59ee791388a595073e4aafbb040af47abdec3d11cdc1508a82c9b6b03788754b`，快速槽 `save860.dat` 为 `270aa5a3a61c7109def87ea6375eb7458c77fdc177c3e491f1cdcd27a05cd1dc`。

## 2026-11-01：原生OP、全屏往返与游戏内退出

继续使用已验证的原生软件绘制/音频构建与隔离用户目录，`--exit-after 600` 仅作有界兜底。实际鼠标从标题MEMORIES进入其他鉴赏并播放OP；窗口连续显示不同视频画面直至尾段，随后自然回到其他鉴赏页并恢复bgm01，没有点击跳过。日志记录 `op.cmv` 音轨段offset=242579572、size=2152846及播放位置推进至120107ms；没有人工听音或音画同步质量结论。另用既有 `cmvs-cmv <op.cmv> --check-frames` 成功解码全部3074帧（21288ms），没有新增测试。

返回标题的系统页后，实际切到2560×1440无边框全屏，再恢复原963×571窗口；日志分别为 `native fullscreen=true/false`，控件继续可点击。随后QUIT先选否回设置，再次QUIT选是，脚本正常请求退出，进程退出码0、输入丢弃0、用户设置保存成功，配置 `FULLSCREEN=0`；窗口清单确认已关闭。全屏切换后捕获有一个15×15辅助目标，误用该目标的点击被工具拒绝，刷新并选实际全屏画面后完成操作；没有将失败点击算作游戏问题。

原始输出仍忽略：`native-window-04.log` SHA-256 `7c0b9956b6aa4f137f280a8aaeb56b3403ed807cfd75e60ac435a2c4f12fc9cb`，`op-all-frames.log` 为 `9f170447bc86edd257b96886d318f2b305a89c545fd2ab98b097f43571aaae41`。本轮不修改运行时，研究放行及未支持快照边界不变。运行指南新增本作可直接执行的启动命令，避免沿用已经不需要的禁用快照参数。

## 2026-11-01：鉴赏键盘导航限制定位

对实际omake页读取列表状态：侧栏list0为 `disable_navigation=false, keep_input=true`，缩略图list1为 `disable_navigation=true, keep_input=false`。因此此前探针尝试键盘选择组7第0项时，缩略图保持 `current=None`，反复发送的下一项却改变侧栏选择；不是缩略图缺失，也不能用鼠标点击冒充纯键盘通过。原始状态记录 `local/chronoclock/gallery-nav-flags.log`。

探针现在对明确禁止导航的列表，在排入任何输入前返回 `keyboard navigation is disabled for group 7`；不改游戏状态或强制解除禁止标志。实际同页复验记录 `gallery-nav-denied.log`：拒绝后页面签名不变、输入队列为空。普通列表及剧情键盘输入策略不变；既有严格Clippy与格式检查通过，未新增测试。缩略图需使用鼠标，这项限制保留并给出具体原因。

## 2026-11-01：全检查结果与I1页面误识别修正

此前启动的 `playflow-all-v2` 已正常结束（探针退出码1），不是挂起或超时。实际构建为 `94e4f43`，包含列表控件、v2快照及序列空闲修正，不含后续滚轮边。参数 `--checks all --fork save --allow-presumed --allow-unreviewed --max-steps 300000000`，鼠标、种子0、16ms、MS Gothic，输出独立用户目录。32项中30项通过：S1/S2/S4/S5、R1–R5、D1–D3、I2–I5、P1–P4、C1–C4、V1–V5、L1；S3按默认闸门静态报告130条脚本命令会停止，此结果不因研究放行而改为通过。

R4四轮分别4/5/6/6条路线全部回标题、无未探索前缀，第四轮无新发现而收敛。R3执行711627 / 2613348条静态指令（27.2%）、涉及67脚本；P1为12页面、290次尝试、无重放偏离且未截断。P4的单次通关前后页面数8/20，不代表全部鉴赏资源都已检查。总报告 `summary.json` SHA-256 `874ccbcb1ad88437b42c008549000a08c0ff7642371cdf6b079586f04ce3dc1b`。这份报告保留原结果，未事后改写。

I1失败来自新接入列表后探针的观察方式：滚轮上实际打开回想页，页含组7、8，旧I1却调用剧情选择驱动器，报 `ambiguous-choice`。改为动作后先运行90帧，再直接观察消息等待或稳定页面；不要求页面只有一个选择组，也不注入剧情驱动器的兜底确认。引擎及选择探索的歧义检查不变。

新release仅复跑相关I1/I3/I4，本作研究策略与《アマツツミ》默认策略均通过，I1分别如实记录推进、同一等待、两组页面等效果。本作 `input-page-chrono/summary.json` SHA-256 `5bb71612652cab5efdad8c90b3f5ab4c36b0b09b119c6d3f76fd4ce438a1ae0e`；基准 `input-page-base/summary.json` 为 `d8034fdecf8699be89f181c870b652db759f2ac3eac67c8dbd6fe611cc9d9240`。既有11项探针测试、严格Clippy及格式检查通过，未新增测试。未用这次局部复跑宣称整套全检查已在最新构建重跑。

## 2026-11-01：回放收敛与鉴赏返回

沿用 `2032575` 的探针库、记录宿主与两项研究放行，用忽略目录的一次性驱动器验证真实脚本流程；未添加仓库测试。初始CSS1仍取已完成路线的 `half-sine-unlocks/users/276-route/system.dat`，所有写入在新隔离目录，没有手改解锁位。

回放初始有6项及返回按钮。单独跑完这6项后发现每一段实际开放对应的下一段，因此不能把初始6项算成全回放。随后用同一会话，每次按当前实际开放的按钮ID选择尚未走过的一项，自动推进消息等待、持有正常跳过，直到返回回放页再继续。最终ID10–21共12段、6770个消息等待均结束并返回；页面从7控件增至13控件，无新增未走条目，取消回到标题。记录 `local/chronoclock/gallery-replay-converge.log` SHA-256 `c9387efa5fc9676451e29b8bf47a1c85d42679740dcaacc9c95ec68a9759feae`。这是当前种子下游戏自然开放回放的收敛，不将脚本返回视为逐帧音画对照。

CG页通过面板内滚轮逐段取得可点击项，62个缩略图均实际进入不同于列表的查看器；每次取消后页面签名与进入前相同，最后返回标题。记录 `gallery-cg-all.log` SHA-256 `14c553c4512e65b64d83350aebb48a3e4f605c1aeb53ad669fbc463ebd352004`。未逐一遍历每张CG的全部差分，也未宣称纯键盘缩略图导航限制已解决。

其他鉴赏页的OP入口经过8072帧返回原页；6个自动演示入口分别经过21、15、15、15、22、20个消息等待，均回到原页面签名，再取消均回标题。记录 `gallery-other-01.log` SHA-256 `4f6eef345868726d9c4261b575f3bf8da9fa269db02cc7136663dcfb54293a51`。OP使用记录宿主，此处不验证原生视频/音轨质量。以上操作没有运行时停止；全检查长任务仍独立进行，未重复启动或提前报告全部通过。

## 2026-11-01：原生窗口与保存重启操作

代码基于 `2032575`，离线构建 `cargo build --offline --release --features desktop,audio-output --bin cmvs-desktop`；使用Windows软件绘制窗口、MS Gothic、Shift-JIS与原生音频，两项研究放行保持开启。原游戏目录只读；隔离用户目录 `local/chronoclock/native-user-01` 的CSS1来自已完成解锁路线，初始剧情槽来自此前V1–V4输出，未手改解锁位。通过Windows窗口实际鼠标操作，未以探针动作替代原生输入。

第一次普通沙箱启动在品牌画面后退出码1，日志没有具体原因，未据此诊断引擎错误。随后在允许桌面设备访问的执行环境运行两次，均退出码0：第一次标题SYSTEM把后台运行改为停止，返回标题LOAD读取种子槽，推进一句，从工具栏打开Data Save，在空槽001保存并确认；返回剧情后滚轮打开回想、右键关闭，文字保持；点击窗口关闭按钮正常退出并保存设置。新 `save000.dat` 为152809字节，离线CSV2校验及VM状态解析通过，脚本为 `snky01.ps3`、PC `0xd6a0`。未覆盖原有种子槽。

第二次使用同一用户目录重启，设置界面仍选后台停止，配置文件 `ACTIVE_DRAW=0`；LOAD选择新槽001并确认后恢复到同一句「むっとする湿気と日差しに眉をひそめながら、待ち人だった石田秀二が屋上に足を踏み入れる。」。有界运行恰在读档后结束，最后一次窗口抓取未取得；宿主正常导出的最终帧与执行日志确认恢复，不能把不存在的窗口截图列为证据。失效窗口句柄已停止使用。

原生音频日志记录192000Hz双声道输出设备打开，标题bgm37、读档后bgm20以及单声道语音 `sysnky01001.ogg` 流启动成功；此为后端实际启动证据，没有人工听音或音质结论。画面、鼠标及存档重启仅覆盖上述流程，GPU后端、全鉴赏及全部剧情仍未验证。

输出均在忽略目录：`native-window-02.log` SHA-256 `d147fb1224daedb427c176142687369cefc9151d3f9c02a24468e18aad6ac778`，`native-window-03.log` 为 `74bd301f8e68f933063ec33f7ac79e68ec0fc969dba9af8a20d3cca8e7ede778`，最终帧 `native-window-03.png` 为 `16ab096aee620a110838eb5b364acbad8772fcbfd4700df1fff2f3c76517a262`，新槽 `save000.dat` 为 `49b03fac1484ca21be7afb607c8278238fa5dcf729d0c7befc9673df9784d4d5`。本轮仅补实际操作记录，没有新增测试或修改运行时。

## 2026-11-01：鉴赏分类、滚动与音乐操作

沿用上一节已完成解锁路线的隔离CSS1种子与两项显式研究放行，不修改游戏解锁位。新增通用 `WheelAt` 抽象动作，鼠标先位于列表命中区域再发出滚轮；页面探索对列表组加入上下滚轮边。旧动作保留原有指针位置，避免改动既有回想操作。所有坐标从运行中列表的命中区域取得，不写游戏特定常量。

一次性实际流程结果：CG页探针组7向下滚5次后，可点击项从0–19变为15–29，页面身份变化；向上滚5次后，原页面身份及0–19可点击范围完全恢复。分类组6的0–6项均能切换、无运行时停止。原始记录 `local/chronoclock/gallery-scroll-check.log`、`gallery-categories-check.log`，CPU页面输出各在同名前缀目录。此处验证分类和滚动操作，不表示全部62个缩略图均已打开。

音乐鉴赏列表实际有34项控件。进入时播放bgm01；再次按第0项后 `playing=false` 且曲名清空；第1、2项分别切换到bgm02、bgm03，`playing=true`；取消后回到7按钮标题，并播放标题的bgm37。记录 `local/chronoclock/gallery-music-play.log`。这是脚本、CPU画面与记录音频宿主状态验证，不宣称已听到原生音轨，也不将未解锁曲目视作已验证。

既有探针测试和严格Clippy通过，未新增测试数量。启动中的 `playflow-all-v2` 全检查使用上一提交的探针，尚不含本次滚轮边；其结果须按实际构建解释。回放结束/返回和原生窗口、音轨仍待推进。

## 2026-11-01：解锁轮次完成与列表控件接入

此前启动的 `local/chronoclock/half-sine-unlocks` 长任务完成：P4、V5、R4、R5均pass L1，L1及R3为pass L0。R4共4轮、21条路线后收敛，首轮4条、后续5/6/6条，无未探索前缀；标题从4按钮变为7按钮，第二轮开始还识别出新的剧情入口。R3执行711379 / 2613348条静态指令（27.2%），涉及67个脚本。R5第二遍由62869帧降为30465帧。报告SHA-256 `911dfc36f556abea1c79b100062d8e58a0ba453ee9fd9476bc3933b4b347cbd6`。该可执行文件在v2快照和两次探针修正之前启动，使用旧无快照/兜底推进策略，也不观察列表组；这些限制不随源代码后来变化而自动消失。

解锁后的旧入口超时不是运行时卡死：以已完成的 `users/276-route/system.dat` 的只读副本进入标题，按钮4打开hscene列表组0（7项），按钮5打开omake列表组0（10项）及组1（62项）。旧PageSignature只枚举六个普通组，所以把实际页面看成零按钮。一次性运行与CPU画面保存在忽略的 `gallery-entry-*` 目录。

现在探针使用统一组号：0–5普通组，6–13列表组0–7。页面观察、鼠标点击、键盘当前项观察、路线入口、页面动作、随机输入和资源计数均接入列表组；列表命中矩形加上绑定对象的2D/字体坐标，并与组的命中区域相交，完全不可见的条目不作可点击项。列表页面身份保留有效布局和实际轮询PC，以区分分类/滚动状态及仍分配列表对象的图片查看器，不含游戏特定脚本/地址常量。页面稳定要求没有挂起VM命令，避免在入场等待尚未结束时点击。

最终代码的一次性鼠标验证 `local/chronoclock/gallery-settle-check.log`：omake首次稳定于 `0x3acd0`、pending none；点击探针组7第0项实际打开CG，查看器与缩略图页身份不同；取消后页面身份完全恢复。Hscene组6第0项实际启动 `snhmk01.ps3` 回放脚本。此为CPU输出与脚本状态验证，未对照原生画面，也未走完所有回放。纯键盘可到omake入口，但试选组7第0项未能通过现有链接导航到达，作为限制保留，不能声称全部键盘鉴赏操作通过。

既有探针测试、Clippy及格式检查通过，未新增测试数量，仅在已有稳定页面测试中补入一个挂起命令参数。较早的列表版本已完成P1–P4/C1–C4，但尚未包含查看器身份和等待边界修正；最终版本的全检查另行运行。后续仍需验证滚轮、所有分类及缩略图、音乐与回放返回流程，不能以列表页面被识别代替这些操作的完成。

## 2026-11-01：D2定时序列误推进纠正

旧D2在第8个等待出现33ms多一句，逐帧研究确认不是引擎丢失文字：两次运行的回想都含序号8200的台词。16ms下脚本在 `snky01.ps3 / 0xe524` 持续轮询0x2281的活动序列；探针将300帧无等待误当作空闲，发送确认，脚本据此用0x2284提前结束序列并越过该句等待。33ms下序列在300帧前结束，因此报告两条轨迹不一致。不注入确认时，16ms在第392帧自然到达该句0x2153的repeat=2。局部逐帧证据在忽略的 `local/chronoclock/waits-repeat-16.log`，两种帧时的回想/等待记录为 `waits-trace-{16,33}.log`。

最终仅在通用场景驱动器观察到 `SequenceHeader.active != 0` 时重置空闲计数；不把活动序列当作确认循环，也不使用游戏脚本名或PC作分派条件。未保留显字状态实验，未改核心运行时、序列语义、闸门或按键配置。

新release探针本作研究策略运行D2、I1–I5、V1–V4全部通过（I1/I5为L0，其余L1）；D2的16/33ms前40个等待完整一致，存读档仍保持恢复轨迹与字节往返。报告 `local/chronoclock/sequence-nudge-chrono/summary.json` SHA-256 `92f94d8f445eee140c06e26349f20f95ffa911cd3667541cf0035f0abb44e890`。同可执行文件对《アマツツミ》默认策略的D2/I2/I4全部通过，报告 `sequence-nudge-base/summary.json` 为 `8d0e5378ea6ed779f1891801b6f84aabbc9d820776f6ce2a977d84ddf29b2c29`。既有探针测试、严格Clippy、格式检查通过，没有新增测试。先前长解锁任务使用旧探针，其结果需注明旧驱动策略；未宣称全路线已经用新策略重跑。

## 2026-11-01：存档分叉验证与回想检查纠正

使用上一节v2存档实现的独立release探针，`--checks R2,R3 --fork save --allow-presumed --allow-unreviewed --max-steps 300000000`，同一游戏根目录、字体、种子0、鼠标、16ms，输出 `local/chronoclock/snapshot-layout-fork`。4条路线 `0.0.0 / 0.1.0 / 0.1.1 / 0.0.1` 全部回到标题，与此前重放得到的选择路径相同；后三条实际从深度1、2、2的检查点恢复，而非退回整条重放。报告SHA-256 `cee51c0001b827736bf62dfab2d0841596d5bcddd50dba588456a650d5859fff`。这验证初始选项树的快照捕获与分叉，不代替解锁后全部入口验证。

旧I4“回想关闭后多推进一句”是探针误判。一次性逐帧记录表明：进入回想前同一句的等待已从repeat=2持续到62；关闭后返回同一脚本/PC、相同文字摘要，repeat继续到71及以后。原检查只认repeat=2，在返回的持续等待中找不到新等待，于是场景驱动器空闲300帧后自行发送推进，再把下一句判作失败。现在回想返回检查接受挂起0x2153且repeat>=2的持续等待，新等待事件的识别保持不变；未改运行时输入或消息语义，也未保留无助于解释该问题的输入队列实验。

以新探针分别对本作研究放行及《アマツツミ》默认策略运行I4，均pass L1。两边前后文字摘要分别保持 `08e1ae3f1d59`、`c21744a80a1a`。本作报告 `local/chronoclock/backlog-wait-chrono/summary.json` SHA-256 `c9864f8f96e99b33cbef75d8e473abefd8db55975503c07dcba6414999fee762`；基准报告 `local/chronoclock/backlog-wait-base/summary.json` 为 `2a6c81a7f027e6dab129a79d2cdbfeba14fac33c038bd6dd354c9c2679e4c6a8`。逐帧记录在忽略的 `local/chronoclock/backlog-trace-1.log`。复用既有探针测试与严格Clippy，未新增测试。D2的33ms多一次等待现象仍独立保留，不能据此标为解决。

## 2026-11-01：剧情存档v2与恢复流程

按独立核对的文字v2子记录启用本作受支持的快照布局，详见[语义证据](vm-semantics.md#剧情快照文字v2布局2026-11-01)。保留未知派生布局、特殊图像及活动场景效果等错误边界，不改变命令闸门与研究放行策略。默认离线API仍使用基准v3，调用方须显式选择v2；runner按档案选择。

release库重建后以同一探针main在 `local/chronoclock/snapshot-layout-probe.exe` 链接独立可执行文件（长流程仍占用旧探针exe）；同一游戏根目录、字体、鼠标、种子0、16ms，`--checks V1,V2,V3,V4 --allow-presumed --allow-unreviewed --max-steps 300000000`，新输出 `local/chronoclock/snapshot-layout-saves`。四项全部pass L1：V1新会话重启读档与V3同会话读档后，20个消息等待轨迹相同，开头均多恢复存档点一句（lead=1）；V2再次保存的快照/脚本映像/辅助段逐字节相同；V4覆盖槽999和读取前槽998备份的头可解析。实际输出save003的两个文字记录（槽7、10）均为版本2，各321字节。该轮只验证剧情开头指定等待点，未宣称全部原版存档或全路线存读档兼容。

报告 `summary.json` SHA-256 `182e036fe788ff6753ab1c4fb1d2bd7370c86709fd4b43415f294de8e9389d4c`。该族检查不输出逐帧事件，`events.jsonl`为空。原游戏目录只读，所有文件写入探针隔离用户目录。同一新可执行文件对《アマツツミ》默认策略执行V1–V4也全部pass L1，输出在 `local/chronoclock/snapshot-layout-base-saves`。

复用现有测试，未新增测试数量：工作区离线测试949项通过、17项外部样本测试忽略；core全目标严格Clippy通过。从真实save003导出的101366字节快照以显式v2解码后重新编码逐字节一致，默认v3解码拒绝其文字记录（一次性检查留在 `local/chronoclock/snapshot-layout-roundtrip.log`）。整条路线的快照捕获和存档分叉另行运行，不将上述开头存读档检查扩展为全剧情结论。

## 2026-11-01：半波运动解开初始选项树

沿用显式 `--allow-presumed --allow-unreviewed`，实现 `0x21fa` 轨道的半波Y模式 `0x22` 与同分支别名 `0x31`。原版两构建的分支、振幅、时间及计算顺序见 [语义证据](vm-semantics.md#半波模式-0x22--0x312026-11-01)。没有改档案状态、未支持模式错误或派生快照屏障。

release 探针，游戏根目录与字体同下方设置入口推进一节，`--checks R2,R3 --max-steps 300000000`，默认鼠标、种子0、16ms，独立输出 `local/chronoclock/half-sine-routes`。R2通过：4个选项点、4条路线全部回到标题，无未探索选项。路径 `0.0.0 / 0.1.0 / 0.1.1 / 0.0.1` 分别执行103450700、128279774、125447672、113300502步；此前 `0.0` 停在 `snky24b.ps3 / 0x1e8f0`，现在越过该点并展开后续选择。R3通过：执行586237 / 2613348条静态指令（22.4%），涉及57个脚本；这不等同于完整游戏覆盖率。报告 `summary.json` SHA-256 `daf413ace7659069c9eac546ff6c7f56557d08a222f08afca3ec3e699858be43`，事件 `events.jsonl` 为 `404f881850d7cf3d9705b7081ffa2280eebd5ca3836b020edc041c6add505556`。

仅扩充原有运动测试参数表，场景/序列回归共61项通过、1项外部样本测试忽略，core全目标严格Clippy、格式及diff检查通过。仍需解锁轮次、通关后重启/页面、已读跳过与长路线计数检查；初始选项树通过不代表全部解锁入口或原生画面、音轨及剧情存读档通过。

## 2026-11-01：研究放行下的设置入口推进

实现 `0x22ff` 的其他显示器使用设置查询，证据见 [VM 语义](vm-semantics.md#0x22ff其他显示器使用设置查询2026-11-01)。保持档案状态与快照格式屏障，继续使用 `--allow-presumed --allow-unreviewed`。本轮不表示其他未复核命令获得语义验证，也未实现原版多显示器切换。

release `cmvs-probe`，游戏根目录 `E:/Download/Purple Software/2015-04-24_CHRONOCLOCK`，字体 `C:/Windows/Fonts/msgothic.ttc`，`--checks P1,P2,P3,C1,C2,C3,C4 --max-steps 300000000`，默认鼠标、种子0、16ms，输出独立新目录 `local/chronoclock/other-display-probe`。七项全部通过：P1/P3/C1 为 L0，P2/C2/C3/C4 为 L1。页面图12节点、290次尝试、无重放偏离、未截断；标题设置入口越过旧 `intproc.ps3 / 0x8163a / 0x22ff` 停点。

C2 修改 `active_draw` 为 false，正常保存 `system.dat`、`cmvs.cfg` 后以新会话重启，值仍为 false；C3 修改文字速度后，前10个等待的帧数从1020变为1382；C4 再按同一控件恢复原设置。游戏目录只读，所有写入在探针用户目录。报告 `summary.json` SHA-256 `695942dd10a94ddd690e593c83c83c31c476c2768e1bd0b6cc831210d20470e4`；页面探针当前不记录事件，故 `events.jsonl` 为空，细节以报告为准。原生窗口、实际显示器切换、全剧情与剧情存读档不在本轮验证范围。

检查：平台与 runner 既有测试（含一个新增命令行为测试）共69项通过，core 全目标严格 Clippy、格式及 diff 检查通过。下一步仍是剧情路线 `0x21fa` 的未支持运动模式。

身份和建档时的结构检查见[游戏档案](README.md)，逐条语义见[VM 证据](vm-semantics.md)。原游戏只读，真实日志、存档副本和探针仅在忽略的 `local/chronoclock/`，不进入 Git。未修改资源解码算法，未引入外部源码或依赖。

## 2026-10-31：对象可见性与オートデモ

在 IDA 中重新核对输入哈希、`0x2026` 的 switch、处理函数、对象 setter/getter、初始化和绘制队列调用。另用只读 PE/Capstone 核对基准 x64 对应完整函数，接入共用的 `SceneObject::shown`。未进行 P2 全表指纹继承，档案状态及默认闸门保持不变。

### trace

沿用建档命令，加 `--allow-unreviewed`：种子0、帧16ms、预算100万步、读取原目录 CSS1/剧情槽目录、RecordingBackend、`--continue-without-snapshot`。现在完成首条 `cdemo.ps3 / 0x341a2 / 0x2026`，随后在 `intproc.ps3 / 0x4021a / 0x2141` 保持挂起（日志编号43920）。trace 没有接入 MessageHost；`0x2141` 已在完整 runner 实现，这个工具停点不能当作缺失游戏指令。

### 完整 runner

仓库外素材通过 `Startup` 正常启动，自动按数据/二进制识别档案。研究参数：virtual_tick=0、随机种子0、每次 Yield 后 `advance_frame(16)`、记录文字、`ContinueWithoutImage`、显式允许未复核命令、无输入、预算100万次 tick。使用 `local/chronoclock/runner-user/` 独立用户目录，初始 CSS1 来自原 `save/system.dat` 副本；不启用本作未登记的 skip-auto-resume 策略。

沿标题空闲进入 `cdemo.ps3`，经过公共脚本消息窗口处理，执行 `0x2026` 三次：`0x341a2`、`0x342d4`、`0x342a8`。在 steps=102011、`cdemo.ps3 / 0x1ef16 / 0x21fa` 明确报错；六参数见 VM 证据。这里的 steps 属于完整 runner，不与 trace 的编号直接比较（回调与宿主调度不同）。

这是无窗口记录运行；没有原生音轨、字体/画面对照、剧情路线、选择或结局通过的结论。CHRONOCLOCK 的全局初值仍未单独复核，也未用基准初值补零。下一步应研究 `0x21fa` 的 `0x00140041` 模式，再逐步复核所经过命令；默认运行仍停在首个未复核命令。

### 输出身份

| 忽略目录内文件 | SHA-256 |
|---|---|
| `visibility-trace.log` | `3a6007d894100e6a1e5c74297555202857b595dfab4efa065912fe3018a13a23` |
| `runner.log` | `f2f1e33da12366683c4b34d832bc4041281de83c82dc6dd747e8fd61756a8825` |
| `visibility-base-disasm.log` | `3e951e8d5c916fc91667440ccd505168c900c12dde14bf7d3b420276c8019561` |

### 检查

`cargo fmt --check`、`cargo test --offline`、`cargo clippy --offline --all-targets -- -D warnings` 和 `git diff --check` 通过。默认测试合计831项通过、12项忽略；新增三项可见性测试仅用合成脚本和像素。未改后端/feature，未新增原生设备验证。

## 2026-10-31：单周期Y摆动与显示字节计数

基线 `9fb0fd2`，分支 `feat/chronoclock`。IDA仍为已登记的CHRONOCLOCK x64，重新确认输入哈希。核对 `0x21fa` 的键写入和帧更新模式65，并以基准exe独立反汇编确认高16位振幅/单周期360度Y偏移。新增该分支后，完整runner到steps=102479、`cdemo.ps3 / 0x10396 / 0x2119`。

继续核对 `0x2119` 的字符串解析、有效文字对象前置条件、字节/注音/控制码计数及返回；基准构建完整函数对照一致。实现后同一真实命令返回22，runner到steps=102522、`cdemo.ps3 / 0x10afe / 0x2112` 停止。当前停点是布局测量命令，不能按计数命令或现有 `0x211f` 直接放行。

保持前轮的种子0、virtual_tick=0、16ms帧、无输入、记录文字、ContinueWithoutImage和显式允许未复核命令；使用相同来源CSS1副本和独立用户目录。最终日志 `text-count-runner.log` SHA-256 `c33ba4691a40b372d796849c5c234309acd8bfa771348c235bc1839af8c3957d`。仅在出错前取得模式65的1个活动帧（slot20、elapsed16、duration5000、mode0x00140041、基准Y=36），Python独立f32/f64模拟与原始XYZ位一致；未执行整个轨道周期或新增原版画面对照。基准反汇编日志 `motion-base-disasm.log` SHA-256 `46f7d8e39280d47f00580648908983e71f1e34a06917c8d78690b46740b31513`。

资源解码未改，无新依赖或外部源码；档案仍为712条Unreviewed，默认闸门保留。新增测试全部为合成数据。当前工作不包含原生窗口、字体音轨输出、完整剧情或全表复核。

检查：`cargo fmt --check`、`cargo test --offline`（833项通过，12项忽略）、`cargo test --offline -p cmvs-core`（56项通过，1项忽略）、`cargo clippy --offline --all-targets -- -D warnings` 通过。新增核心单测1项、消息集成测试2项，扩展既有轨道命令参数覆盖。

## 2026-10-31：回到P2，完整指纹与默认闸门验收

先将前述语义改动提交为`8eb8708`，本阶段以其为基线，只改工具、档案与闸门。实现IDAPython导出、标准库比较、带强制证据的`Inherited`解析/验证、状态统计及闸门来源计数；具体策略和命令见[设计第13节](../../design/engine-profiles.md#13-p2-实施与使用2026-10-31)。全局/间接依赖、未知解码、取地址/索引数据和递归截断均不自动继承。

IDA 9.4分别导出两个构建的x64/x86，二进制身份来自各自档案，原exe只读。CHRONOCLOCK x64使用当前IDA会话，其他三份使用`local/chronoclock/p2-*.i64`新建分析库，未切换当前会话或改写原游戏目录。工具核对输入文件哈希并拒绝数据库补丁；同一CHRONOCLOCK x64重新导出逐字节一致。原指令图JSON只留local，仓库报告仅含指纹和分类。

| 比较 | 完整匹配 | 未复核 | NoOp | 报告 |
|---|---|---|---|---|
| x64对x64（档案采用） | 387 | 325 | 264 | [x64 TSV](profiles/cmvs-20150405-x64.tsv) |
| x86对x86（独立核对） | 362 | 350 | 264 | [x86 TSV](profiles/cmvs-20150405-x86.tsv) |

报告头保留归一化策略、深度8、两边exe和导出JSON的SHA-256；档案以`inherit x64 base-sha256=`头部行绑定根exe，继承行只记架构和完整指纹，派生exe取本档案同架构`binary`行。x64报告SHA-256 `e867b33857fda781f9f40e56cea2792ee213a79c16531766341cdd5b60905a4e`，x86报告 `fe472ddd98df13a92655e43ad82f5d6e0b4bd70a3b932245d04e246966aa865f`（LF；首次生成时工具在Windows上写出CRLF，对应哈希为`62495b45…`/`7e5c6d0e…`，内容逐行相同，工具已改为强制LF）。x86不同/截断不等于语义不同，不直接合并两份结果。

### 默认运行

采用建档trace参数（种子0、帧16ms、原CSS1只读、RecordingBackend、ContinueWithoutImage、百万步预算），**不加`--allow-unreviewed`**。`start.ps3 / 0xc / 0x201e`因完整匹配通过，在`start.ps3 / 0x1a / 0x201f`报未复核，退出码1，栈顶10与PC保留。统计为Base=0、Inherited=1、Unreviewed=0；统计包含等待重试的分派尝试，不能当作已完成指令数。未复核原因是可写全局依赖，下一步P3应人工复核它，不能直接沿用基准状态。

默认日志`p2-default.log` SHA-256 `13dfd56569b83b66f1d17242a28259aa825caddf37fb163d3c6ecd3104de372b`。先前`0x2112`是研究放行路径的历史停点，不能替代默认兼容状态。

### 基准回归与检查

从Git导出`8eb8708`到忽略目录独立构建，与P2版本同参数运行《アマツツミ》trace：virtual_tick=0、种子0、16ms、百万步、RecordingBackend、ContinueWithoutImage，输入CSS1为`local/amatsutsumi/user-data/system.dat`（SHA-256 `dc20174a630f6525d70d6c4452896c716249ef2c469adf472bf6053ac0e5ef1a`），剧情槽扫描同目录；输入不写回。两份日志完整121639651字节逐字节一致，SHA-256均为`9d8e45d8e8aa5dc14202fb460f953acad04cd5f02d1d93ad83397d17bb48ca21`；两边都在第1000000步预算耗尽，退出码1。日志在`p2-amatsu-before-saved.log`、`p2-amatsu-after-saved.log`。游戏原目录没有CSS1时两版也一致早停未知变量，该早停不计入百万步验收。

检查通过：`cargo fmt --check`、默认离线测试834项（12项忽略）、core离线测试56项（1项忽略）、默认/core严格Clippy、Python标准库合成指纹测试10项（含候选档案头部行）。未新增命令语义、依赖或原生设备验证；`Reviewed`/`Variant`及剩余差异人工复核仍属于P3。

## 2026-10-31：M1，`0x201f` 人工复核

按[分派层设计](../../design/dispatch.md) M1 引入 `Reviewed`。默认停点 `0x201f` 的指纹原因是可写全局（`unresolved-data`、`unresolved-or-mutable-data`）。CHRONOCLOCK x64 在当前 IDA 会话只读查询；基准 x64 用 IDA 9.4 `idat` 对 `local/chronoclock/p2-amatsu-x64.i64` 的临时副本批处理读取，两边输入哈希与档案一致。结论与地址表见 [VM 语义](vm-semantics.md)：处理函数相同；全局只被归档类工厂读取，取值 1、2 在基准中有专用类而本作走默认，取值 10 两边都构造 0x1C8 字节的归档类；Rust 只记录该值不解释，故登记为 `Reviewed`。

默认运行参数同上节。停点前移到 `start.ps3 / 0x50 / 0x2091`，退出码 1，统计 base=0、inherited=3、reviewed=1、unreviewed=0。日志 `local/chronoclock/m1-default.log` SHA-256 `130018b6d499460b16f7bbe16bdc619e69e6d11697d7c0906c35d129f102d5a5`；去掉统计与错误行后，P2 日志是它的逐字节前缀。

《アマツツミ》同上节参数重跑，日志 `local/chronoclock/m1-amatsu.log`（LF）SHA-256 `4803fe2eff8f8e1d48730847e4b94f69f6e6af356b05c2580808a72f48b28d90`，与 P2 的 `p2-amatsu-after-saved.log` 去掉 CR 后的哈希相同。P2 记录的 `9d8e45d8…` 是经 PowerShell 重定向写出的 CRLF 版本，内容一致。

`0x2091` 的工具原因为未建模/可写数据引用，留待下一条复核。

## 2026-10-31：指纹策略 v2（运行库叶子）

按[档案设计第 14 节](../../design/engine-profiles.md#14-指纹策略-v22026-10-31)改进导出与比较工具。先在当前 IDA 会话只读分类 v1 中被数据问题阻断的 469 个函数，再离线模拟各项修复的效果；运行库边界（上限 58 条）加段外引用（上限 78 条）收益最高且无需新的配对推理，故先做这两项。

导出均为 IDA 9.4 `idat` 批处理：基准 x64/x86 读取 `local/chronoclock/p2-amatsu-*.i64` 的临时副本；CHRONOCLOCK x64 用 `-o` 新建分析库 `local/chronoclock/v2-chrono-x64.i64`（不再借用当前会话），x86 读取 `p2-chrono-x86.i64` 的临时副本。游戏目录除当前会话自身的 `cmvs64.exe.id0` 外无文件变化。导出 JSON 的 SHA-256：

| 导出 | SHA-256 |
|---|---|
| 基准 x64 | `cf7a9adce3a229fc61fe52c093a5dc7a2b4bc6376aa532db405d9d1a739a0c48` |
| CHRONOCLOCK x64 | `bab7f8b73ac6f238cb8bc575d1b4ba6b9ee61d2e18a5d434c7f6553d2927ee21` |
| 基准 x86 | `d9dbab950e5a890c049e9a2e73843fedad98740d1a8bf9fb523539a04de53bf0` |
| CHRONOCLOCK x86 | `2a90040813148256aa259534950be37fbd1292a5fdb4168c3e1c87d762bf60a1` |

[比较报告](profiles/)替换为 v2 版本：x64 `161a628bcdb1bfcadfee4483e7a27e8eed7a40033befa1cee33aaeb6fd677933`，x86 `e27c5081ea55fcee04b87a38f46097335347dcd19e15cd81a36471409b8d43bb`。x64 继承 387 → 434，x86 362 → 415，v1 的继承无一丢失。第一次导出时有一对占位名库函数（`sub_…`、`unknown_libname_N`）被当作叶子；它们因名称不同从未匹配，但编号可能偶然相同，故改为按普通函数展开并在比较工具中拒绝，重新导出后继承数不变。

默认 trace 同上节参数，仍停在 `start.ps3 / 0x50 / 0x2091`，除统计行外与 M1 日志逐字节一致（`local/chronoclock/v2-default.log`）。Rust 代码未改，只更新本作档案数据，故未重跑《アマツツミ》百万步回归。检查：`cargo fmt --check`、默认离线测试 836 项（12 项忽略）、严格 Clippy、Python 合成测试 15 项。

## 2026-10-31：复核辅助 `--explain`

见[档案设计第 14 节](../../design/engine-profiles.md#复核辅助---explain)。带诊断字段的 x64 导出为 `local/chronoclock/v2d-*-x64.json`（基准 x64 读 `p2-amatsu-x64.i64` 临时副本，CHRONOCLOCK x64 读 `v2-chrono-x64.i64` 临时副本）；与已提交报告逐行相同。`0x2091` 的解释：处理函数 `0x7f1fa` → `0x6eb60` → `0x82330`（基准 `0x82b61` → `0x72100` → `0x85ce0`），在 `+0x32` 读取可写全局（CHRONOCLOCK `qword_140131ea0`），`+0x15` 的映像基址 `lea` 位移不同。人工复核从这两处开始。

## 2026-10-31：P3，`0x2091` 位变量初始化复核

基线 `763a74e`。沿默认闸门停点人工核对两个 x64 构建的完整处理函数、位写入函数、错误标志指针 setter 和唯一构造调用者；两边 IDA 输入哈希均与档案一致。CHRONOCLOCK 使用当前会话只读查询，基准由 IDA 9.4 批处理读取 `p2-amatsu-x64.i64` 的新副本 `review-2091-base.i64`。原游戏保持只读，分析库和输出仅留在忽略的 `local/chronoclock/`。

两边对合法范围逐位写入的语义相同，差异为数据区和代码地址；可写全局实际指向 VM `+12056` 错误标志，位区另有逐字节同步的冗余校验副本。现有 Rust 根语义保留变量值，不模拟副本失配标志；非法范围继续明确报错。证据与边界见 [VM 语义](vm-semantics.md#位变量范围初始化-0x20912026-10-31-复核)。仅将 `0x2091` 从 `Unreviewed` 改为 `Reviewed`，不改指纹算法、指纹报告或运行时代码。当前统计为 Inherited=434、Reviewed=2、Unreviewed=276、NoOp=264。

### 默认启动验收

使用 debug `cmvs-trace`，建档命令原参数：`--bootstrap`、原游戏 `save/` 和 CSS1 只读、`--script-archive`、`--record-platform`、`--continue-without-snapshot`、帧16ms、种子0、百万步预算；**不加 `--allow-unreviewed`**。第16步的 `start.ps3 / 0x50 / 0x2091` 以 start=0、count=65536、value=0 完成，回收12字节并前进至 `0x52`。第23步在 `start.ps3 / 0x76 / 0x2092` 明确报未复核，PC和12字节参数保持挂起；其参数为 start=0、count=2048、value=0。闸门统计 base=0、inherited=3、reviewed=2、unreviewed=0。

对 CR 规范化并去掉旧日志末尾统计及错误行后，`v2-default.log` 是新日志的逐字节前缀。没有新增原生窗口、字体、音轨或完整剧情验证；研究放行的 `0x2112` 停点仍为历史结果。下一步复核 `0x2092`，不能据位写入相同直接放行整数初始化。

| 忽略目录内证据 | SHA-256 |
|---|---|
| `review-2091-chrono.json`（反汇编与伪代码） | `b9337987c169147fdc56ef1f8abd3026601968e6b48a0babf7eee28472843372` |
| `review-2091-base.json`（反汇编、伪代码和指针引用） | `fa22a5fe2837226d1f1c093a644f2f22b5d16164f31dea98f566d26e9f86a0da` |
| `review-2091-default.log` | `8179bc6b3882e3a0c38e7b8d394d77125fdedcbe685f2ffb386f2a0efab185be` |

检查通过：`cargo fmt --check`、`cargo test --offline`（836项通过、12项忽略，含复核文档引用及现有合成初始化测试）、`cargo clippy --offline --all-targets -- -D warnings`、`git diff --check`。未修改解码算法、后端或 feature，无新依赖及外部源码；不重复计入《アマツツミ》百万步回归，本轮只改派生档案数据与文档。

## 2026-10-31：指纹策略 v3 与 `Presumed`

见[档案设计第 15 节](../../design/engine-profiles.md#15-指纹策略-v3-与-presumed2026-10-31)。先在当前 IDA 会话只读统计：间接控制流中 switch 跳转 24 处、间接调用 247 处；switch 信息可由 IDA 完整给出。随后逐步核对导出：`lea reg, [映像基址]` 未建模、IDA 未解析的映像相对表访问（如 `0x2091` 的 `movzx ecx, byte [r8+rdx+131EB0h]`）、不返回调用后的 `int3` 填充被当作缺失后继。映像相对位移只在 IDA 已解析或同一基本块内可证明寄存器装有映像基址时置零；该证明在全部 CHRONOCLOCK x64 函数中认定 140 处，抽查均为跳转表与索引表，没有结构体偏移。

导出用 IDA 9.4 `idat` 顺序批处理（并行启动时两次出现数据库输入名读取异常，改为顺序执行后正常），数据库均为 `local/chronoclock/` 中分析库的临时副本：

| 导出 | SHA-256 |
|---|---|
| 基准 x64 | `7d1f49d573c1ef912cf05e40c12c3a6dd136c6465c596f56784f140f50a7fdae` |
| CHRONOCLOCK x64 | `5a3cefbac5ff268e28bcce9cc17baf246051b42c53548a639734d8a876e9742a` |
| 基准 x86 | `88bd9b1d2c0eee29fb339582621cf1edc1e42ddf719b2a015ccddf424bd25f59` |
| CHRONOCLOCK x86 | `cc888234a0efb9c0289c6badb131e9cc56c4ca5f9f6ac22e57fb00a9836b6665` |

[比较报告](profiles/)：x64 `d9a0afbc53cc9688741c99e46505d9a16b18e00ed2595650dc80c29b6ab34e8d`，x86 `1aee44ae1d1b51bfe537d59f419987fec7c8d41ac6135bab0482dbb4de63f8e6`。档案由工具从当前档案生成，保留 `0x201f`、`0x2091` 两条人工复核行。

默认 trace（参数同上）停在第 43 步 `start.ps3 / 0xde / 0x2013`，统计 inherited=4、reviewed=2、presumed=3（`0x2092`–`0x2094`），日志 `local/chronoclock/v3-default.log` SHA-256 `29cc6581ee28eb35c5b1a21c6cce091a5f5cb4b2d654daa5f966b54c3816cc11`。`0x2092` 的解释显示可比较部分全部相同，阻断的只有整数区、其校验副本和错误标志指针三处可写数据，与 `0x2091` 复核所见的结构一致。

`--explain` 逐条查看研究路径上剩余的未复核命令：`0x2010`、`0x2012`–`0x2014`、`0x2019` 的首处差异在归档类工厂 `0x1400273d0`，`0x2011` 在 `0x140034260`，`0x201a` 在 `0x1400362f0`，都是 `0x201f` 复核中记录的取值 1、2 差异；`0x2030`、`0x2141` 为对象字段偏移不同，`0x2080` 有多处常量不同，`0x2128` 的首处差异是一处未证明的映像相对地址。

《アマツツミ》百万步 trace 与 M1 日志逐字节一致。检查：`cargo fmt --check`、默认离线测试、严格 Clippy、Python 合成测试 21 项。

## 2026-10-31：P3，七条归档路径注册命令复核

复核 `0x2010`–`0x2014`、`0x2019`、`0x201a`，全部由 Unreviewed 登记为 Reviewed；档案现为 Inherited=455、Presumed=167、Reviewed=9、Unreviewed=81、NoOp=264。不改运行时、指纹策略、导出或自动比较报告。各命令参数、返回值、管理器及工厂地址见 [VM语义](vm-semantics.md#归档路径注册组2026-10-31-复核)。

CHRONOCLOCK 使用当前 IDA 会话只读查询，输入 SHA-256 与档案一致；基准用 IDA 9.4 批处理读取新副本 `review-archives-base.i64`，输入 SHA-256 亦核对一致。完整处理函数均取槽号和字符串、拼接根目录、传入两个校验开关，返回 `0x4008`。三个工厂的类型1、2分支沿用 `0x201f` 证据，但这不是全部差异：图像工厂的路径数组/指针数组从基准 +4232/+6280 移至本作 +2152/+4200。记录步长2056、槽数10、析构/替换和校验配置操作一致，Rust路径映射不依赖原版内存布局。另两种工厂接受五槽（0..4）；读取器当前只消费音频/影片前四槽的限制保留。

为避免首差异遮住后续依赖，只在两份导出的内存副本中将三对已人工核对的工厂替换成相同叶子，用现有v3 shape诊断：七条命令的工厂外可比较部分全部一致；剩余阻断来自共享字符串解析器可写全局字符串区。随后人工核对该解析器的标签、槽数、步长和越界回退，以及CPZ6构造与vtable +40校验setter。此诊断不是完整指纹继承，不改变生产算法或其他命令状态；其他归档类内部、后续加载及原生输出未验证。

### 默认启动验收

debug `cmvs-trace`，仍用建档参数：`--bootstrap`、`--script-archive`、`--record-platform`、`--continue-without-snapshot`、`--frame-ms 16`、`--rand-seed 0`、`--steps 1000000`，不加 `--allow-unreviewed`。本次将原CSS1复制到新的 `local/chronoclock/review-archives-input/system.dat`，`--save-dir` 与 `--system-save` 均指向该独立目录，原游戏只读。副本运行前后 SHA-256 均为 `39a93d66c3020f95f6eca644d4e1a79c9318d0f8f53366a9b7a408f5b2fa5385`；游戏档案记录原目录无剧情槽，因此没有新增剧情槽输入。

原第43步 `start.ps3 / 0xde / 0x2013` 完成后推进到 `0xe0`，共13次归档路径注册全部完成。第295步停在 `start.ps3 / 0x11f8 / 0x2128`，退出码1，PC和8字节参数保持挂起；统计 base=0、inherited=5、reviewed=15、presumed=12、unreviewed=0（含等待重试的分派尝试）。执行过的推定命令为 `0x2015`、`0x2016`、`0x2092`–`0x2094`、`0x2098`、`0x22b3`。去除旧日志末尾统计、推定列表及错误行并规范化换行后，v3默认日志为本次日志的相同前缀。

| 忽略目录内证据 | SHA-256 |
|---|---|
| `review-archives-base.json`（完整反汇编、伪代码） | `2290027b9872d0ce9fc3648c2f85105f68ac09173cf30acf77cadcdcdbd9cda5` |
| `review-archives-chrono.json`（完整反汇编、伪代码） | `54342643438d6cf050f9987abd90269011e4320dc71ec07a6c2c5cca005a3d0a` |
| `review-archives-compare.log`（工厂边界诊断） | `daf041e00580b03697e767c7058becaf4a7b5d45cd65f2b10d5ddc085b65a344` |
| `review-archives-default.log` | `0b859806f8e42716c9d915ab922f3ddffccc60695ad3f310b26947acb948a67d` |

研究路径还剩四条未复核命令（`0x2128`、`0x2080`、`0x2030`、`0x2141`），下一步按默认停点复核 `0x2128`。没有新增原生窗口、字体、音轨或完整剧情验证，也未改基准档案及运行代码，因此本轮不重复计入《アマツツミ》百万步回归。

检查通过：`cargo fmt --check`、`cargo test --offline`（836项通过、12项忽略，含逐命令复核小节引用检查）、`cargo clippy --offline --all-targets -- -D warnings`、`git diff --check`。新旧默认trace经LF规范化后的5553字节前缀完全一致。本轮不新增外部来源、依赖或解码算法，不需新增真实解码对照；原始研究输出与分析库继续留在忽略目录。

## 2026-10-31：P3，原采样路径剩余四条复核

`0x2128`、`0x2080`、`0x2030`、`0x2141`登记为Reviewed；档案统计为Inherited=455、Presumed=167、Reviewed=13、Unreviewed=77、NoOp=264。各命令的两个x64构建完整哈希、case/函数、参数、返回值、状态、错误路径和支持边界见[VM语义](vm-semantics.md#系统存档读取-0x21282026-10-31-复核)。x86本轮未单独复核，不改独立指纹报告、算法或其状态。

CHRONOCLOCK仍使用当前IDA会话只读查询并重新核对输入哈希；基准从 `p2-amatsu-x64.i64` 新复制为 `review-four-base.i64` 后用IDA 9.4批处理读取，哈希与档案一致。遍历v3依赖树找出全部可见差异，指令不再对齐/调用数不同的分支另按语义配对；未把诊断树当作完整继承证明。

`0x2128`首差异为持久位区及镜像的映像相对地址，已与前轮位区证据核对。`0x2141`差异为文字对象缓冲指针从基准+560移至本作+352，销毁顺序和对象归属不变。`0x2030`深入图像管理器后确认2080字节字段移动、基准新增PB2A分派，以及JBP1跨基本块的映像相对查表地址；查表1024字节逐字节一致，PB3正常分支仍复用已有解码器。没有更改解码算法，PB2A及其他未实现格式继续报错。

`0x2080`主脚本切换和快照实时位0效果一致，但文字记录v2/v3、公共图像8/16槽和效果类型9支持确有不同。本轮不构造伪兼容快照，也不把这些常量抹去；runner新增派生档案快照格式错误屏障，统一阻断其构造/检查和槽恢复。命令复核允许现有切换与无快照策略，不表示已支持本作原生CSV2子记录。新增合成测试验证错误不改变PC、栈和快照图像，且不生成备份槽；根档案存读档由现有合成测试回归。未来派生快照编码/恢复仍需独立证据与实现。

### 默认闸门验收

trace继续使用上一轮全部参数和 `review-archives-input` 独立CSS1副本，不加 `--allow-unreviewed`。越过启动`0x2128`和主脚本切换后，在第43920步 `intproc.ps3 / 0x4021a / 0x2141` 停止；命令已通过闸门，但工具缺少MessageHost。统计inherited=5962、reviewed=24、presumed=2429、unreviewed=0。上一轮默认trace在旧统计行前、LF规范化后的29803字节是新日志的逐字节前缀。

完整runner探针采用 `SnapshotPolicy::ContinueWithoutImage`、记录文字、RecordingBackend、种子0、16ms、无输入、百万tick预算，默认闸门；新建 `review-four-user` 独立用户目录并复制同一CSS1。实际完成 `0x2128` 1次、`0x2080` 4次、`0x2030` 3次、`0x2141` 1次。经main/logo/menu进入cdemo后，steps=82416在 `intproc.ps3 / 0x4027c / 0x215d` 报未复核；前8参数 `[7,10,0,0,1280,54,1280,720]`，继续保持挂起。统计base=0、inherited=15058、reviewed=24、presumed=2429、unreviewed=0，统计仍是分派尝试数。执行过的推定命令完整列表保留在日志。

CSS1运行前后SHA-256均为 `39a93d66c3020f95f6eca644d4e1a79c9318d0f8f53366a9b7a408f5b2fa5385`。原游戏未写入，未测试原生窗口/字形/音轨、剧情快照或完整路线。此前“剩余四条”仅描述旧采样路径；新观察到的`0x215d`不能跳过。下一步仍按新默认停点复核，不启用研究放行追历史`0x2112`。

| 忽略目录内证据 | SHA-256 |
|---|---|
| `review-four-base.json` | `a7ab9681d26c78c1d467f66ec88bd480925edfc1433b848d50a905c3518f968d` |
| `review-four-chrono.json` | `a9c7cd791c613dc457378c6f22afee4ba774efaa0f1a7afb33390ab46d3ae617` |
| `review-four-audit.log` | `646e06e41d2491d309d043c24817354d9c8c5718400b9749ca66bd61878dc6f3` |
| `review-four-image-audit.log` | `427d8d5ed5e8eaa9539d60039b92144db4efddc39885e2f75549d7e53b404e37` |
| `review-four-default.log` | `c12ef9416c0c8781f5f6ca20055cf9151b891f7416f3fd71c7ed4059cbf77a22` |
| `review-four-runner.log` | `5a90f791c4601bb74bf7cabda493396dd7a3a1db2fbfe7466a6f785b6c17dc09` |

检查通过：`cargo fmt --check`、`cargo test --offline`（837项通过、12项忽略）、`cargo clippy --offline --all-targets -- -D warnings`、`git diff --check`。本轮无后端/feature、解码算法、依赖或外部源码变更；没有重新计入根游戏百万步回归，根快照正常行为由现有合成测试覆盖。

## 2026-10-31：P3，按共享差异复核文字/窗口组

基线 `62c9c57`。从新停点 `0x215d` 出发，新增只读诊断 `profile_compare.py --group-differences [命令号…]`，将多个命令的差异函数边界聚合。只沿自身函数体完全对齐的调用边展开，不在不同父体下面按调用次序强行配对；输出包含架构、两边exe哈希和深度，并明确不比较边界后的后代、不改变状态。没有参数时按自动Unreviewed报告选择，因此可能含已经人工Reviewed的命令。工具不引入手工叶子覆盖或字段归一化，指纹策略v3和原比较报告保持不变。

窗口创建处理函数各145条指令，唯一自身差异是文字对象从0x180增至基准0x250字节。人工核对完整构造发现基准多清零208字节扩展块、后续六字渲染参数/工作缓冲/上下文字体设备字段整体移动；共同初始化与后续操作一致。构造的额外memset使调用数不同，按角色重新配对三个子函数继续检查；没有用“首处只差分配大小”代替整个构造复核。与先前析构、快照字段移动形成同一证据组。

共登记九条Reviewed：`0x2140`、`0x215d`、`0x2102`、`0x2118`、`0x214e`、`0x2150`、`0x22e0`、`0x2170`、`0x2176`。后两条复用 `0x2030` 已核对的图像管理器和PB3差异链；其包装函数无新增差异。地址、参数、返回值、支持与错误边界见[共享布局组](vm-semantics.md#窗口与文字共享布局组2026-10-31-复核)。`0x2118`、`0x2176`尚无Rust处理器，Reviewed与已实现分开记录；不为未执行命令补造动态验证。档案现为Inherited=455、Presumed=167、Reviewed=22、Unreviewed=68、NoOp=264。

本作当前IDA输入哈希再次确认，基准从 `p2-amatsu-x64.i64` 新复制为 `review-windows-base.i64` 批处理读取。仅查询反汇编/伪代码，原游戏只读，x86本轮未单独复核。没有新增外部资料、核心语义、解码算法、依赖或原生设备验证；不重复根游戏百万步回归。

### 默认闸门验收与下一组

沿用上一轮runner探针参数：RecordingBackend、记录文字、种子0、16ms、无输入、`ContinueWithoutImage`、百万tick预算，未启用 `--allow-unreviewed`。新建 `review-windows-user` 独立用户目录，复制同一CSS1；运行前后SHA-256均为 `39a93d66c3020f95f6eca644d4e1a79c9318d0f8f53366a9b7a408f5b2fa5385`。

先越过旧 `intproc.ps3 / 0x4027c / 0x215d`，初次验证停在 `0x2170`；补齐这条共用图像证据后，最终到steps=93397、`cdemo.ps3 / 0x2105e / 0x203f`。实际执行本轮新增命令 `0x215d` 2次、`0x2150` 1次、`0x2170` 2次，其余六条只做静态复核。闸门统计base=0、inherited=16305、reviewed=32、presumed=2599、unreviewed=0；这是分派尝试数。已执行推定命令列表完整保留在日志，新增的 `0x2178`、`0x214a`、`0x2033` 等不计作人工复核。

新停点 `0x203f` 的共享差异边界为基准 `0x140035a90` / 本作 `0x1400341f0`；下一轮可聚合 `0x2032`、`0x22c4` 排查全部边界，再决定放行。当前实际推进10981步，不表示完整自动演示、呈现、剧情或存档验证通过。

| 忽略目录内证据 | SHA-256 |
|---|---|
| `review-windows-base.json` | `ebac8f62da872bf822dcb046f3f6f1220479ff019e2993d1c50f169c7f5893c4` |
| `review-windows-chrono.json` | `bae42a3411f7b1cc078ac2a4e3f7a82082bd7a86544ac2daa4a1caf1b11805dc` |
| `review-windows-audit.log` | `21c5f4b2017e115e5a9aa454b519ad9195b5c1a72e389ac724ef7b02eb4508e3` |
| `review-windows-helpers.log` | `e511ba7d441dc628cc2fb719106d232cdbd3e7a69b078981a81fe2f3b96bc7cf` |
| `review-windows-groups.log` | `bb38acb8cdfb0fa20ec4670015ac9aeb757b373fc807ef5c19ca705ab3a2a9d2` |
| `review-windows-runner.log` | `d660951f9d3d1adc64e6304086ed98a602555a92d2aec50c7c19d3ad472e6978` |

检查：Python合成测试25项通过，新增覆盖共享分组、全部对齐兄弟分支、父体失配停止、深度/参数边界、保守数据及输入不变；`cargo fmt --check`、`cargo test --offline`（837项通过、12项忽略）、严格Clippy和`git diff --check`通过。首次Cargo测试遇到既有资源测试按时间戳命名目录的并行撞名，已加进程内原子序号并完整重跑通过；不改变资源读取行为。

## 2026-10-31：P3，图像缓存提示与换图组

从默认停点`0x203f`继续，按共享差异聚合`0x2032`、`0x22c4`，复核三条并登记Reviewed。当前Inherited=455、Presumed=167、Reviewed=25、Unreviewed=65、NoOp=264。运行时、指纹策略、自动比较报告与解码算法不变；没有新增依赖或外部源码。参数、返回标志、成员对照与实现限制见[图像缓存组](vm-semantics.md#图像缓存提示与换图组2026-10-31-复核)。

本作当前IDA会话输入哈希核对一致；基准从`p2-amatsu-x64.i64`复制到新的`review-cache-base.i64`，用IDA 9.4批处理只读查询，输出身份核对一致。提示位/查询状态分别移动2080字节；预加载结果取得函数58条指令、12处成员位移变化；原始读取166条指令、5处字段/路径位移变化。缓存查询/插入调用树可比较部分相同，格式分派后按资源签名配对，复用先前`0x2030`证据，不按失配父函数下的调用序号判等。未知格式与未建模模式继续显式报错。x86未独立复核。

### 默认闸门验收

复用上一轮完整runner探针，RecordingBackend、记录文字、种子0、每次Yield推进16ms、无输入、`ContinueWithoutImage`、百万tick预算，未启用`--allow-unreviewed`。新建`review-cache-user`并复制CSS1，运行前后SHA-256均为`39a93d66c3020f95f6eca644d4e1a79c9318d0f8f53366a9b7a408f5b2fa5385`。原游戏保持只读。

两次`0x203f`在`cdemo.ps3`的PC `0x2105e`、`0x1ddf6`完成，分别记录steps=93397、93406；本轮其余两条未动态执行，`0x2032`仍无处理器。新默认停点steps=102340、`cdemo.ps3 / 0xf090 / 0x22b2`，比旧停点推进8943步。闸门分派尝试统计inherited=17179、reviewed=36、presumed=2600、base/unreviewed=0，含等待重试，不是完成数或覆盖率。已执行推定命令列表保存在日志，不额外计为人工复核。没有验证原生窗口、文字/音轨、剧情快照或完整自动演示。

下一步复核`0x22b2`的实时状态更新与快照调用，仍保留派生构建快照格式屏障；没有以研究放行绕过该停点。初步诊断涉及快照子记录数量及状态数组差异，不能把它视作单一字段移动。

| 忽略目录内证据 | SHA-256 |
|---|---|
| `review-cache-base.json` | `ffda00334aa7cd5af9df162e61f450540030ca08734e7ec4b07b551c718cf130` |
| `review-cache-chrono.json` | `b876bd25d2a5311544eee0950619d2880dbc4182cc2165db36ca9b401f43966a` |
| `review-cache-audit.log` | `12b17e660c5ed5cfc53fa816328570354f699b6d4dc1cdcf276226734142186f` |
| `review-cache-runner.log` | `3a7c93e19e2f0563a8cb5e5f22241f9405f4a4048232df741082ca4b864ecf10` |

检查：`cargo fmt --check`、`cargo test --offline`（837项通过、12项忽略）、`cargo clippy --offline --all-targets -- -D warnings`通过。无后端/feature或解码变更，不重复计入根游戏百万步重放。既有合成测试覆盖缓存与档案文档引用；未增加与数据改动重复的测试。

## 剧情点与基本布局推进（2026-11-01）

M3 已提交 `e5dfb8d`，本轮回到真实运行停点，不扩大契约框架。使用当前 CHRONOCLOCK x64 和同一基准 exe，完整身份见[语义记录](vm-semantics.md#0x22b2剧情点快照请求2026-11-01-复核)。IDA 当前输入哈希再次核对；基准用既有 v3 导出和原 exe 逐指令绑定，未修改 IDA 数据库或游戏文件。

1. `0x22b2`：两边 case/包装各15条指令绑定通过，包装仅快照调用地址不同。PC临时加2、重复计数清0、快照后减2、返回 `0x4000` 一致。再次核对目标快照构造中的位0置1/复制/清0，复用 `0x2080` 已有的格式差异证据。登记实时语义为Reviewed；新增 runner 测试确认默认Capture仍拒绝派生格式并保持待处理，显式ContinueWithoutImage才完成命令并标记保存图像不可用。
2. 通过后在步骤102522、`cdemo.ps3 / 0x10afe / 0x2112` 遇到缺少处理器。实现基本布局查询：结果0/1/4为行数、最大行字节数、末行字节数；复用原有注音预扫描，不把它套成`0x211f`。两边处理函数/布局/lead helper分别38/176/11条指令一致（地址重定位），加case各229条指令绑定原exe。核对包装的临时注音标志缓冲仅16字，超过范围的原栈覆盖显式拒绝。本条也登记Reviewed。

重放参数：正常入口、种子0、16ms、记录文字、无输入、ContinueWithoutImage；每次新建独立用户目录，CSS1从原模板只读载入。显式选择 `allow-presumed=true`、`allow-unreviewed=false`，严格策略仍会在步骤24的旧推定`0x2092`停止。本轮没有通过放宽未复核命令绕过停点。

实际观察：

- `0x22b2` 在步骤102340、`cdemo.ps3 / 0xf090`完成，PC到`0xf092`、SP=160、repeat=0、位0=false，保存图像为Unavailable。
- `0x2112` 在步骤102522、PC`0x10afe`完成，槽7、字符串区9的22字节文本（11个双字节字符）；区域`[30,45,360,45]`、光标`[30,30]`、字号30、间隔0。逐字符以右界360检查，11次各前进30且均无需换行，独立计数为`[1,22,22]`，与系统整数0/1/4一致。PC到`0x10b00`，SP=220（只回收8字节），未提交快照。
- 新停点步骤105242、`cdemo.ps3 / 0xfdb4 / 0x2157`明确报未复核；相对旧停点增加2902步。栈顶`[0,58000,11071]`。来源分派尝试：继承17440、人工复核40、旧推定2645，契约/未复核0；含等待重试，不是完成数或覆盖率。

`0x2157` 的分组首差异是输出函数基准`0x14005bc90` / 本作`0x140059000`（577/499条指令）。基准有额外禁则 helper 调用，不能只凭文字尾字段移动放行；本轮保留未复核，下一步逐分支核对输出链。没有额外执行`0x2157`、宣称文字像素正确或走完整部自动演示。

原游戏CSS1运行前后SHA-256均为`39a93d66c3020f95f6eca644d4e1a79c9318d0f8f53366a9b7a408f5b2fa5385`。快照格式、存读档、原生窗口/输入/音轨和整部路线验证范围未扩大。

| 忽略目录内证据 | SHA-256 |
|---|---|
| `review-scenario-chrono.json` | `0105130a1164f326de71604e7ee560fabc6bdd43d65779c4e8f3bc093e4c2bae` |
| `review-layout-amatsu-export.json` | `5ed445fb0eeee1a8741e4dc57a1afb9488dea90ee9099bdb239fc368b35f2fa5` |
| `review-layout-chrono.json` | `828916c773104eeadee0b7c4ec8f4ac43ca0b5975bae44ac90c61e5a1c7ea79b` |
| `scenario-layout-final.log` | `b9243aa90d01d53311c8a2a991e9fc866594bce6aa2e8121e0f5e6fd7245f6de` |

新增三项基本布局合成测试覆盖换行/注音/控制码、结果槽与文字/下层栈保留、字符串区引用、16标志边界和可重试错误；原有108项message测试继续通过。另有一项派生剧情点的格式屏障/显式研究继续回归。当前27条Reviewed、163条Presumed、64条Unreviewed；其余状态不变。

检查：`cargo fmt --all --check`、`git diff --check`、默认846项/全workspace全功能930项测试通过（另12/22项默认忽略），默认及全workspace全功能严格Clippy通过。未增加依赖、改动资源解码器或启用派生剧情快照；没有把运行步数增加当作画面/音轨/整部路线验证。

## 实际输入推进与可玩性停点（2026-11-01）

上一轮运行代码已提交为 `4103212`。本轮按用户授权显式启用 `allow-presumed=true`、`allow-unreviewed=true`，不改变命令证据状态；仅用忽略目录内的临时探针运行，不将游戏特化探针或 Cargo 入口纳入仓库。沿正常入口、种子0、16ms虚拟帧推进，安装字体度量并采样渲染文字/场景，启用影片帧解码。平台请求仍记录；音频请求流式记录到本地文件，不代表真实播放验证。快照仍选择 ContinueWithoutImage，未绕过派生存读档格式屏障。

实际输入与结果：

- 标题 START 为组0、ID1，区域 `[490,372,300,60]`；等待标题过渡后以悬停、按下、释放进入新游戏。普通鼠标路径累计1780次消息点击，在一亿步预算内进入 `snky03.ps3`，未遇到处理器或采样渲染错误。空闲演示循环不计作剧情推进。
- 原配置 `READ_SKIP=1` 下 Ctrl 停于未读文本属于预期策略。另开全新独立用户目录，复制配置并仅将 `MESSAGE_INFO/READ_SKIP` 改为0，之后持续 Ctrl；选择出现时释放 Ctrl，经悬停、按下、释放选择首个启用选项。原游戏配置未写入。
- 快进路径经过 `snky01` 至 `snky07`，选项后进入 `snky21`、`snky22`、`snky23`、`snky24a`、`snky24b`；两次选项分别在步骤22815899、38924426注入。最终步骤40651750、15939虚拟帧，在 `snky24b.ps3 / 0x1e8f0 / 0x21fa` 报 `world motion interpolation mode is not implemented`，退出码1。采样渲染9次，最终画面可见背景和立绘；未核对原生像素。
- 途中两处探针自身限制已排除：影片播放时间被误算为等待，导致影片结束当帧提前退出；默认录音后端4096条请求上限导致长剧情中止。前者仅修临时探针计时，后者改用本地流式记录后继续。没有为这两处修改游戏运行时。

本轮日志 `local/chronoclock/probe-route-a.log` SHA-256 为 `21b0f2a0e694d0b68ce5f4590511230f9314c8197523da1d0fcaee3e092de921`；画面位于同目录的 `probe-route-a/final.png`。原游戏 `save/system.dat` 哈希仍为 `39a93d66c3020f95f6eca644d4e1a79c9318d0f8f53366a9b7a408f5b2fa5385`。这些本地证据不随仓库分发。

下次优先调查 `scene/world_motion.rs` 对 `0x21fa` 插值模式的拒绝，先复核当前脚本参数与本作实现，再补所需行为。根据用户额度要求，本轮到此收束；尚未验证整条路线、所有选项、真实音轨、原生窗口输入或存读档完整度。
