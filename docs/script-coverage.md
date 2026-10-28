# 脚本覆盖审计（2026-10-29）

本报告保留 b05ee17 提交前的审计快照。后续已实现 `0x0105`，鉴赏入口的新停点为 `omake.ps3 / 0x368f8 / 0x2371`；本表历史 PC 数量、OMAKE 失败记录及证据哈希不随之改写，最新进展见 [审计缺口](script-audit.md)。

本报告由 `local/script_coverage.py` 从静态元数据与明确分类的重放 PC 日志生成。

- 结构通过：88/88 个来源（包内 87 个、包外 1 个）。
- 本次重放实际服务过指令的来源：87/88。
- 去重指令地址：1055703/2957772（35.69%）。
- 静态系统命令 ID：431；本次服务过：250。

指令地址指标包含等待命令被成功服务的地址，不代表命令已经完成。分母包含重复公共库、不可达或未调用代码，不能解释为游戏完成度。没有采集条件跳转的两侧边，因此没有分支覆盖率。本次只有 Ctrl、记录文字/音频与无窗口重放，不增加画面/原生音轨验证。

`start.ps3` 的运行时名称映射到包外启动来源；包内同名脚本单独统计。各来源的地址仅对本次哈希所标识的数据有效。CPZ 资源校验和通过，头部/索引摘要未验证。

## 本次路线

| 路线 | 服务过的去重地址 | 结束状态 |
|---|---:|---|
| KOKORO | 341506 | `success=true steps=93141973 yields=27737 stop=title ready choices=1` |
| KYOKO | 448329 | `success=true steps=125809467 yields=32582 stop=title ready choices=2` |
| AI | 599341 | `success=true steps=171510141 yields=39515 stop=title ready choices=3` |
| HOTARU1 | 737989 | `success=true steps=213849807 yields=56711 stop=title ready choices=4` |
| HOTARU2 | 738668 | `success=true steps=211977387 yields=42578 stop=title ready choices=4` |
| OP | 2116 | `success=true steps=144756 yields=10322 stop=title ready choices=0` |
| OMAKE | 1862 | `success=false steps=4608 yields=567 stop=error omake.ps3:36172 code+0x36174 opcode=0x0105: operand storage/type is not implemented` |

## 逐脚本清单

“路线”只表示至少一个地址被服务，不表示整份脚本或画面全部通过。

| 来源 | 代码字节 | 静态指令 | 已服务地址 | 标签 | 系统 ID | 路线 |
|---|---:|---:|---:|---:|---:|---|
| cmovie.ps3 | 222178 | 24850 | 98 | 198 | 221 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| intcode.ps3 | 198674 | 14515 | 8893 | 21 | 4 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2, OP, OMAKE |
| intproc.ps3 | 569388 | 67170 | 2459 | 352 | 390 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2, OP, OMAKE |
| logo.ps3 | 222852 | 24970 | 180 | 199 | 223 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2, OP, OMAKE |
| main.ps3 | 235208 | 26257 | 146 | 203 | 254 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2, OP, OMAKE |
| menu.ps3 | 234322 | 26507 | 3471 | 198 | 227 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2, OP, OMAKE |
| omake.ps3 | 330082 | 38400 | 7 | 222 | 267 | OMAKE |
| op.ps3 | 222058 | 24816 | 92 | 198 | 221 | OP |
| sn1001.ps3 | 369980 | 50131 | 29468 | 199 | 219 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1002.ps3 | 329430 | 43229 | 23204 | 198 | 219 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1003.ps3 | 316642 | 40984 | 20483 | 198 | 219 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1004.ps3 | 337930 | 44658 | 24478 | 198 | 219 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1005.ps3 | 287678 | 36069 | 14822 | 198 | 219 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1006.ps3 | 289346 | 36355 | 15597 | 198 | 219 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1007.ps3 | 272186 | 33422 | 12448 | 198 | 219 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1008.ps3 | 365944 | 49440 | 28609 | 199 | 219 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1009.ps3 | 288610 | 36249 | 15151 | 198 | 219 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1010.ps3 | 353364 | 47298 | 26568 | 199 | 219 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1011.ps3 | 237654 | 27514 | 5792 | 198 | 222 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1012.ps3 | 311902 | 39648 | 18397 | 203 | 244 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1501.ps3 | 255718 | 30596 | 9485 | 198 | 219 | KOKORO |
| sn1502.ps3 | 268946 | 32856 | 12025 | 198 | 219 | KOKORO |
| sn1503.ps3 | 279456 | 34672 | 13590 | 199 | 219 | KOKORO |
| sn1504.ps3 | 294096 | 37164 | 17822 | 199 | 219 | KOKORO |
| sn1505.ps3 | 248146 | 29315 | 8249 | 198 | 219 | KOKORO |
| sn1506.ps3 | 300952 | 38336 | 17574 | 199 | 219 | KOKORO |
| sn1507.ps3 | 256800 | 30789 | 10202 | 198 | 219 | KOKORO |
| sn1508.ps3 | 237612 | 27499 | 6060 | 198 | 219 | KOKORO |
| sn2001.ps3 | 305228 | 39015 | 19414 | 199 | 219 | KYOKO, AI, HOTARU1, HOTARU2 |
| sn2002.ps3 | 275302 | 33942 | 13094 | 198 | 219 | KYOKO, AI, HOTARU1, HOTARU2 |
| sn2003.ps3 | 300680 | 38304 | 17764 | 199 | 219 | KYOKO, AI, HOTARU1, HOTARU2 |
| sn2004.ps3 | 258102 | 30986 | 9587 | 198 | 219 | KYOKO, AI, HOTARU1, HOTARU2 |
| sn2005.ps3 | 307332 | 39400 | 18090 | 199 | 219 | KYOKO, AI, HOTARU1, HOTARU2 |
| sn2006.ps3 | 257242 | 30856 | 9808 | 198 | 219 | KYOKO, AI, HOTARU1, HOTARU2 |
| sn2007.ps3 | 254382 | 30363 | 8939 | 198 | 219 | KYOKO, AI, HOTARU1, HOTARU2 |
| sn2008.ps3 | 294184 | 37145 | 16075 | 199 | 219 | KYOKO, AI, HOTARU1, HOTARU2 |
| sn2009.ps3 | 262950 | 31840 | 10597 | 198 | 219 | KYOKO, AI, HOTARU1, HOTARU2 |
| sn2010.ps3 | 243374 | 27913 | 4653 | 202 | 244 | KYOKO, AI, HOTARU1, HOTARU2 |
| sn2501.ps3 | 305604 | 39128 | 18134 | 199 | 219 | KYOKO |
| sn2502.ps3 | 257810 | 30960 | 9770 | 198 | 219 | KYOKO |
| sn2503.ps3 | 241470 | 28173 | 6407 | 198 | 219 | KYOKO |
| sn2504.ps3 | 292912 | 36972 | 15912 | 199 | 219 | KYOKO |
| sn2505.ps3 | 230238 | 26250 | 3779 | 198 | 219 | KYOKO |
| sn2506.ps3 | 281532 | 35014 | 13465 | 199 | 219 | KYOKO |
| sn2507.ps3 | 235012 | 27060 | 5800 | 198 | 219 | KYOKO |
| sn2508.ps3 | 226016 | 25520 | 2008 | 198 | 219 | KYOKO |
| sn3001.ps3 | 323542 | 42207 | 21213 | 198 | 219 | AI, HOTARU1, HOTARU2 |
| sn3002.ps3 | 271338 | 33281 | 12482 | 198 | 219 | AI, HOTARU1, HOTARU2 |
| sn3003.ps3 | 305574 | 39111 | 18653 | 198 | 219 | AI, HOTARU1, HOTARU2 |
| sn3004.ps3 | 335952 | 44310 | 23519 | 199 | 219 | AI, HOTARU1, HOTARU2 |
| sn3005.ps3 | 279390 | 34646 | 13495 | 198 | 219 | AI, HOTARU1, HOTARU2 |
| sn3006.ps3 | 293836 | 37102 | 16271 | 199 | 219 | AI, HOTARU1, HOTARU2 |
| sn3007.ps3 | 279814 | 34716 | 13301 | 198 | 219 | AI, HOTARU1, HOTARU2 |
| sn3008.ps3 | 266102 | 32382 | 9738 | 198 | 219 | AI, HOTARU1, HOTARU2 |
| sn3009.ps3 | 261826 | 31073 | 8807 | 202 | 244 | AI, HOTARU1, HOTARU2 |
| sn3501.ps3 | 261326 | 31562 | 12147 | 198 | 219 | AI |
| sn3502.ps3 | 287644 | 36063 | 15087 | 199 | 219 | AI |
| sn3503.ps3 | 293540 | 37055 | 17245 | 199 | 219 | AI |
| sn3504.ps3 | 248286 | 29334 | 7946 | 198 | 219 | AI |
| sn3505.ps3 | 306748 | 39317 | 18401 | 199 | 219 | AI |
| sn3506.ps3 | 264912 | 32164 | 10874 | 198 | 219 | AI |
| sn3507.ps3 | 246124 | 28965 | 6896 | 198 | 219 | AI |
| sn4001n.ps3 | 281078 | 34946 | 15576 | 198 | 219 | HOTARU1, HOTARU2 |
| sn4002n.ps3 | 274990 | 33900 | 12388 | 198 | 219 | HOTARU1, HOTARU2 |
| sn4003n.ps3 | 279818 | 34731 | 13531 | 198 | 219 | HOTARU1, HOTARU2 |
| sn4005n.ps3 | 296270 | 37521 | 16315 | 198 | 219 | HOTARU1, HOTARU2 |
| sn4006n.ps3 | 304354 | 38915 | 18736 | 198 | 219 | HOTARU1, HOTARU2 |
| sn4007n.ps3 | 327544 | 42866 | 21417 | 199 | 219 | HOTARU1, HOTARU2 |
| sn4008n.ps3 | 259658 | 31275 | 9884 | 198 | 219 | HOTARU1, HOTARU2 |
| sn4009n.ps3 | 329564 | 42672 | 20548 | 202 | 247 | HOTARU1, HOTARU2 |
| sn4010h.ps3 | 243492 | 28508 | 6504 | 198 | 218 | HOTARU2 |
| sn4010n.ps3 | 288396 | 36188 | 14442 | 198 | 219 | HOTARU1, HOTARU2 |
| sn4011h.ps3 | 254696 | 30424 | 7792 | 198 | 218 | HOTARU2 |
| sn4011n.ps3 | 264120 | 32045 | 10694 | 198 | 219 | HOTARU1, HOTARU2 |
| sn4012h.ps3 | 246140 | 28967 | 5634 | 198 | 218 | HOTARU2 |
| sn4012n.ps3 | 329242 | 43145 | 21724 | 199 | 219 | HOTARU1, HOTARU2 |
| sn4013h.ps3 | 290088 | 36473 | 13772 | 199 | 219 | HOTARU2 |
| sn4013n.ps3 | 319526 | 41516 | 21238 | 198 | 219 | HOTARU1 |
| sn4014h.ps3 | 263278 | 31916 | 9758 | 198 | 219 | HOTARU2 |
| sn4014n.ps3 | 350702 | 46838 | 24557 | 199 | 219 | HOTARU1 |
| sn4015h.ps3 | 255088 | 30481 | 8654 | 198 | 219 | HOTARU2 |
| sn4015n.ps3 | 234888 | 27030 | 4728 | 198 | 219 | HOTARU1 |
| sn4016h.ps3 | 221758 | 24789 | 348 | 198 | 218 | HOTARU2 |
| staff.ps3 | 228090 | 25784 | 1180 | 199 | 223 | KOKORO, KYOKO, AI |
| staff2.ps3 | 225592 | 25344 | 847 | 199 | 223 | HOTARU1 |
| staff3.ps3 | 223142 | 24995 | 375 | 199 | 223 | HOTARU2 |
| start.ps3 | 4752 | 335 | 0 | 1 | 30 | 未覆盖 |
| 包外 start.ps3 | 4726 | 330 | 320 | 1 | 30 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2, OP, OMAKE |

## 未在本次路线中服务的系统命令

以下是静态出现但本次未服务的 ID，不等同于未实现；已服务的命令也可能仍有未实现的参数/模式。

`0x2001`, `0x2003`, `0x2008`, `0x202e`, `0x2034`, `0x2035`, `0x203e`, `0x2065`, `0x2069`, `0x206c`, `0x2078`, `0x208c`, `0x2099`, `0x20a9`, `0x20aa`, `0x20af`, `0x20c8`, `0x20ca`, `0x20d0`, `0x20f1`, `0x20f3`, `0x20fc`, `0x2106`, `0x2130`, `0x2131`, `0x2132`, `0x2133`, `0x2135`, `0x213d`, `0x2140`, `0x214c`, `0x214d`, `0x2154`, `0x2157`, `0x2158`, `0x2159`, `0x2163`, `0x2164`, `0x216b`, `0x216c`, `0x216d`, `0x216e`, `0x216f`, `0x218d`, `0x2194`, `0x2195`, `0x2196`, `0x219a`, `0x219c`, `0x219d`, `0x219e`, `0x219f`, `0x21a3`, `0x21a8`, `0x21a9`, `0x21aa`, `0x21ab`, `0x21ac`, `0x21ad`, `0x21ae`, `0x21af`, `0x21e3`, `0x21e4`, `0x220c`, `0x221a`, `0x221b`, `0x2226`, `0x2227`, `0x2229`, `0x222a`, `0x222b`, `0x2234`, `0x2235`, `0x2238`, `0x2239`, `0x223a`, `0x223b`, `0x223c`, `0x223d`, `0x2260`, `0x2261`, `0x2262`, `0x2263`, `0x2264`, `0x2287`, `0x2289`, `0x228c`, `0x228e`, `0x22a0`, `0x22a2`, `0x22a8`, `0x22ae`, `0x22b5`, `0x22b6`, `0x22b7`, `0x22b8`, `0x22ba`, `0x22bb`, `0x22bd`, `0x22c4`, `0x22c5`, `0x22c8`, `0x22c9`, `0x22ce`, `0x22d3`, `0x22d4`, `0x22d5`, `0x22d6`, `0x22d7`, `0x22d8`, `0x22d9`, `0x22da`, `0x22db`, `0x22dc`, `0x22dd`, `0x22de`, `0x22df`, `0x22e0`, `0x22e1`, `0x22e4`, `0x22e7`, `0x22e8`, `0x22e9`, `0x22ee`, `0x22f8`, `0x22fc`, `0x22fd`, `0x230b`, `0x230c`, `0x2321`, `0x2322`, `0x2328`, `0x2329`, `0x2344`, `0x2346`, `0x234a`, `0x234e`, `0x234f`, `0x2353`, `0x2359`, `0x235c`, `0x2360`, `0x2365`, `0x2366`, `0x2367`, `0x2368`, `0x2369`, `0x2370`, `0x2371`, `0x2372`, `0x2373`, `0x2374`, `0x2375`, `0x2376`, `0x2377`, `0x237c`, `0x237d`, `0x237e`, `0x2380`, `0x2382`, `0x2383`, `0x2385`, `0x2386`, `0x2387`, `0x2389`, `0x238b`, `0x238c`, `0x238e`, `0x238f`, `0x2390`, `0x2392`, `0x2393`, `0x2396`, `0x2397`, `0x2398`, `0x239a`, `0x239b`, `0x239d`, `0x239e`, `0x239f`, `0x23a0`

## 证据身份

游戏数据、PC 日志、重放探针与输出保留在忽略目录或游戏目录，Git 只保留元数据报告。

| 输入 | SHA-256 |
|---|---|
| `local/script-audit-sites.tsv` | `8fac5f6b980ebfef96e4eb61c3414674f58537f1201536978b75a22b59f20f41` |
| `E:/Download/AMATSUTSUMI/data/pack/script.cpz` | `92b367e8c3d2791f1dd2ca8426581974fbb640d2766f2e82ab12383d2f5359e4` |
| `E:/Download/AMATSUTSUMI/data/pack/start.ps3` | `c338313949a803e834acf0d746c93f09c4d41f558af8fb6c13ab1ee55aff2bd0` |
| `E:/Download/AMATSUTSUMI/cmvs64.exe` | `dc74d2001387ae1aefbdc4204216de5ebb9d3896009280720c4e7a91b42d84f5` |
| `E:/Download/AMATSUTSUMI/save/system.dat` | `ace328a04d320626ee61ef69e1668d90e1c7a2a862e5fbd8c0a2dfa92f089410` |
| `local/audit-replay.exe` | `d7fc4bed24283ec4297269d55d87ef8da55680b0ca9e0521693867b6f0594e7e` |
| `local/audit-targeted.exe` | `ab0ea53c0b2c30424715da76c8aebc05a19075d79ff5bb72cf2c4a7609e9a0a6` |
| `local/audit-final.exe` | `3e4a7f471f7cc5d03462a276621c2d08aa8039891b9c9dc40f7d7aa8dbf1e7b2` |
| `local/audit-extra.exe` | `174481c165f9077fd8b168489f3c1d287c9f7efe6e80b0b35a4bc6f497e31380` |
| `local/audit_replay.rs` | `72ccbc43667ea72a68b1782352cd666fb0e69778af937b47cae0c5c63a1fd9ba` |
| `local/audit-first2.log` | `752ce672368e38fda8077fa60d7b650e8badd1692ca0d5cf2a1ee6856e1b47ac` |
| `local/audit-first2-pcs.tsv` | `3334a75834a533b3480337e4ac2baf697643ab6e5618afba90c8397a651463d9` |
| `local/audit-two.log` | `9bfc9d1235753aac4d4367fc7af9e67161914af7e0cd397f61a4becaa9a315db` |
| `local/audit-two-pcs.tsv` | `740f354a38d6e26c538515b89478f536b2cc268e64d46b6941028f8c37dc19a4` |
| `local/audit-three.log` | `c3f7111d90a482aa24baf54e94c74f81f5eb7adc30a31b1954e3d9643fb8440e` |
| `local/audit-three-pcs.tsv` | `81367ca54160982bacd515450b66d4370c6886afab5e1d09091c8d9e4679ed0a` |
| `local/audit-end1b.log` | `c9320b8763f15c345fc69c49a5b211d931c525ca742b47d26a6e4bae727dbadc` |
| `local/audit-end1b-pcs.tsv` | `942ea55e2afaee1589ec43f29cb59a74dfb4d010771010a1f5b90bff9ce7e93f` |
| `local/audit-end2.log` | `09ae07cb0bdfb351d7b7d697df1fd467e1451e17ab8fd70f37b877c45a03aa91` |
| `local/audit-end2-pcs.tsv` | `3e45dbced868af247dfbcf04d46552b6ac7466574a7af951b988cc3b3f3c176d` |
| `local/audit-op.log` | `d7fa1d8ffaa0828ecef8f1b0010dd4134b02326144515a003d33686fbdd80919` |
| `local/audit-op-pcs.tsv` | `dd2f6959dc83c2e5320c8453b256a7ff9b52b1ed231dc2af86fa4b15432be54f` |
| `local/audit-omake2.log` | `c408159e7f1e390174ade447be8d07cf46b7c0e8b11b1a96b42260bd42decb49` |
| `local/audit-omake2-pcs.tsv` | `e6d9487c9360c8cbd5b8906ce22e44aa3d22d427b44c52b4586969c42f6dc767` |
