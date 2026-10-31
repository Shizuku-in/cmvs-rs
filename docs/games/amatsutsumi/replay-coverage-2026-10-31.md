# 脚本覆盖审计（2026-10-31）

本报告由 `local/script_coverage.py` 从静态元数据与明确分类的重放 PC 日志生成。

- 结构通过：88/88 个来源（包内 87 个、包外 1 个）。
- 本次重放实际服务过指令的来源：86/88。
- 去重指令地址：1057150/2957772（35.74%）。
- 静态系统命令 ID：431；本次服务过：268。

指令地址指标包含等待命令被成功服务的地址，不代表命令已经完成。分母包含重复公共库、不可达或未调用代码，不能解释为游戏完成度。没有采集条件跳转的两侧边，因此没有分支覆盖率。本次只有 Ctrl、记录文字/音频与无窗口重放，不增加画面/原生音轨验证。

`start.ps3` 的运行时名称映射到包外启动来源；包内同名脚本单独统计。各来源的地址仅对本次哈希所标识的数据有效。CPZ 资源校验和通过，头部/索引摘要未验证。

## 本次路线

| 路线 | 服务过的去重地址 | 结束状态 |
|---|---:|---|
| KOKORO | 342742 | `success=true steps=150738154 yields=60727 stop=title ready choices=1` |
| KYOKO | 449607 | `success=true steps=204917895 yields=78912 stop=title ready choices=2` |
| AI | 600651 | `success=true steps=282840837 yields=105945 stop=title ready choices=3` |
| HOTARU1 | 739329 | `success=true steps=357847595 yields=143453 stop=title ready choices=4` |
| HOTARU2 | 740028 | `success=true steps=259384057 yields=49073 stop=title ready choices=4` |
| OP | 2139 | `success=true steps=284101 yields=10322 stop=title ready choices=0` |

## 逐脚本清单

“路线”只表示至少一个地址被服务，不表示整份脚本或画面全部通过。

| 来源 | 代码字节 | 静态指令 | 已服务地址 | 标签 | 系统 ID | 路线 |
|---|---:|---:|---:|---:|---:|---|
| cmovie.ps3 | 222178 | 24850 | 98 | 198 | 221 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| intcode.ps3 | 198674 | 14515 | 8893 | 21 | 4 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2, OP |
| intproc.ps3 | 569388 | 67170 | 3604 | 352 | 390 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2, OP |
| logo.ps3 | 222852 | 24970 | 191 | 199 | 223 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2, OP |
| main.ps3 | 235208 | 26257 | 146 | 203 | 254 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2, OP |
| menu.ps3 | 234322 | 26507 | 3439 | 198 | 227 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2, OP |
| omake.ps3 | 330082 | 38400 | 0 | 222 | 267 | 未覆盖 |
| op.ps3 | 222058 | 24816 | 92 | 198 | 221 | OP |
| sn1001.ps3 | 369980 | 50131 | 29472 | 199 | 219 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1002.ps3 | 329430 | 43229 | 23208 | 198 | 219 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1003.ps3 | 316642 | 40984 | 20487 | 198 | 219 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1004.ps3 | 337930 | 44658 | 24482 | 198 | 219 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1005.ps3 | 287678 | 36069 | 14826 | 198 | 219 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1006.ps3 | 289346 | 36355 | 15601 | 198 | 219 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1007.ps3 | 272186 | 33422 | 12452 | 198 | 219 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1008.ps3 | 365944 | 49440 | 28613 | 199 | 219 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1009.ps3 | 288610 | 36249 | 15155 | 198 | 219 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1010.ps3 | 353364 | 47298 | 26572 | 199 | 219 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1011.ps3 | 237654 | 27514 | 5796 | 198 | 222 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1012.ps3 | 311902 | 39648 | 18401 | 203 | 244 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2 |
| sn1501.ps3 | 255718 | 30596 | 9489 | 198 | 219 | KOKORO |
| sn1502.ps3 | 268946 | 32856 | 12029 | 198 | 219 | KOKORO |
| sn1503.ps3 | 279456 | 34672 | 13594 | 199 | 219 | KOKORO |
| sn1504.ps3 | 294096 | 37164 | 17826 | 199 | 219 | KOKORO |
| sn1505.ps3 | 248146 | 29315 | 8253 | 198 | 219 | KOKORO |
| sn1506.ps3 | 300952 | 38336 | 17578 | 199 | 219 | KOKORO |
| sn1507.ps3 | 256800 | 30789 | 10206 | 198 | 219 | KOKORO |
| sn1508.ps3 | 237612 | 27499 | 6064 | 198 | 219 | KOKORO |
| sn2001.ps3 | 305228 | 39015 | 19418 | 199 | 219 | KYOKO, AI, HOTARU1, HOTARU2 |
| sn2002.ps3 | 275302 | 33942 | 13098 | 198 | 219 | KYOKO, AI, HOTARU1, HOTARU2 |
| sn2003.ps3 | 300680 | 38304 | 17768 | 199 | 219 | KYOKO, AI, HOTARU1, HOTARU2 |
| sn2004.ps3 | 258102 | 30986 | 9591 | 198 | 219 | KYOKO, AI, HOTARU1, HOTARU2 |
| sn2005.ps3 | 307332 | 39400 | 18094 | 199 | 219 | KYOKO, AI, HOTARU1, HOTARU2 |
| sn2006.ps3 | 257242 | 30856 | 9812 | 198 | 219 | KYOKO, AI, HOTARU1, HOTARU2 |
| sn2007.ps3 | 254382 | 30363 | 8943 | 198 | 219 | KYOKO, AI, HOTARU1, HOTARU2 |
| sn2008.ps3 | 294184 | 37145 | 16079 | 199 | 219 | KYOKO, AI, HOTARU1, HOTARU2 |
| sn2009.ps3 | 262950 | 31840 | 10601 | 198 | 219 | KYOKO, AI, HOTARU1, HOTARU2 |
| sn2010.ps3 | 243374 | 27913 | 4657 | 202 | 244 | KYOKO, AI, HOTARU1, HOTARU2 |
| sn2501.ps3 | 305604 | 39128 | 18138 | 199 | 219 | KYOKO |
| sn2502.ps3 | 257810 | 30960 | 9774 | 198 | 219 | KYOKO |
| sn2503.ps3 | 241470 | 28173 | 6411 | 198 | 219 | KYOKO |
| sn2504.ps3 | 292912 | 36972 | 15916 | 199 | 219 | KYOKO |
| sn2505.ps3 | 230238 | 26250 | 3783 | 198 | 219 | KYOKO |
| sn2506.ps3 | 281532 | 35014 | 13469 | 199 | 219 | KYOKO |
| sn2507.ps3 | 235012 | 27060 | 5804 | 198 | 219 | KYOKO |
| sn2508.ps3 | 226016 | 25520 | 2012 | 198 | 219 | KYOKO |
| sn3001.ps3 | 323542 | 42207 | 21217 | 198 | 219 | AI, HOTARU1, HOTARU2 |
| sn3002.ps3 | 271338 | 33281 | 12486 | 198 | 219 | AI, HOTARU1, HOTARU2 |
| sn3003.ps3 | 305574 | 39111 | 18657 | 198 | 219 | AI, HOTARU1, HOTARU2 |
| sn3004.ps3 | 335952 | 44310 | 23523 | 199 | 219 | AI, HOTARU1, HOTARU2 |
| sn3005.ps3 | 279390 | 34646 | 13499 | 198 | 219 | AI, HOTARU1, HOTARU2 |
| sn3006.ps3 | 293836 | 37102 | 16275 | 199 | 219 | AI, HOTARU1, HOTARU2 |
| sn3007.ps3 | 279814 | 34716 | 13305 | 198 | 219 | AI, HOTARU1, HOTARU2 |
| sn3008.ps3 | 266102 | 32382 | 9742 | 198 | 219 | AI, HOTARU1, HOTARU2 |
| sn3009.ps3 | 261826 | 31073 | 8811 | 202 | 244 | AI, HOTARU1, HOTARU2 |
| sn3501.ps3 | 261326 | 31562 | 12151 | 198 | 219 | AI |
| sn3502.ps3 | 287644 | 36063 | 15091 | 199 | 219 | AI |
| sn3503.ps3 | 293540 | 37055 | 17249 | 199 | 219 | AI |
| sn3504.ps3 | 248286 | 29334 | 7950 | 198 | 219 | AI |
| sn3505.ps3 | 306748 | 39317 | 18405 | 199 | 219 | AI |
| sn3506.ps3 | 264912 | 32164 | 10878 | 198 | 219 | AI |
| sn3507.ps3 | 246124 | 28965 | 6900 | 198 | 219 | AI |
| sn4001n.ps3 | 281078 | 34946 | 15580 | 198 | 219 | HOTARU1, HOTARU2 |
| sn4002n.ps3 | 274990 | 33900 | 12392 | 198 | 219 | HOTARU1, HOTARU2 |
| sn4003n.ps3 | 279818 | 34731 | 13535 | 198 | 219 | HOTARU1, HOTARU2 |
| sn4005n.ps3 | 296270 | 37521 | 16319 | 198 | 219 | HOTARU1, HOTARU2 |
| sn4006n.ps3 | 304354 | 38915 | 18740 | 198 | 219 | HOTARU1, HOTARU2 |
| sn4007n.ps3 | 327544 | 42866 | 21421 | 199 | 219 | HOTARU1, HOTARU2 |
| sn4008n.ps3 | 259658 | 31275 | 9888 | 198 | 219 | HOTARU1, HOTARU2 |
| sn4009n.ps3 | 329564 | 42672 | 20552 | 202 | 247 | HOTARU1, HOTARU2 |
| sn4010h.ps3 | 243492 | 28508 | 6508 | 198 | 218 | HOTARU2 |
| sn4010n.ps3 | 288396 | 36188 | 14446 | 198 | 219 | HOTARU1, HOTARU2 |
| sn4011h.ps3 | 254696 | 30424 | 7796 | 198 | 218 | HOTARU2 |
| sn4011n.ps3 | 264120 | 32045 | 10698 | 198 | 219 | HOTARU1, HOTARU2 |
| sn4012h.ps3 | 246140 | 28967 | 5638 | 198 | 218 | HOTARU2 |
| sn4012n.ps3 | 329242 | 43145 | 21728 | 199 | 219 | HOTARU1, HOTARU2 |
| sn4013h.ps3 | 290088 | 36473 | 13776 | 199 | 219 | HOTARU2 |
| sn4013n.ps3 | 319526 | 41516 | 21242 | 198 | 219 | HOTARU1 |
| sn4014h.ps3 | 263278 | 31916 | 9762 | 198 | 219 | HOTARU2 |
| sn4014n.ps3 | 350702 | 46838 | 24561 | 199 | 219 | HOTARU1 |
| sn4015h.ps3 | 255088 | 30481 | 8658 | 198 | 219 | HOTARU2 |
| sn4015n.ps3 | 234888 | 27030 | 4732 | 198 | 219 | HOTARU1 |
| sn4016h.ps3 | 221758 | 24789 | 348 | 198 | 218 | HOTARU2 |
| staff.ps3 | 228090 | 25784 | 1190 | 199 | 223 | KOKORO, KYOKO, AI |
| staff2.ps3 | 225592 | 25344 | 857 | 199 | 223 | HOTARU1 |
| staff3.ps3 | 223142 | 24995 | 389 | 199 | 223 | HOTARU2 |
| start.ps3 | 4752 | 335 | 0 | 1 | 30 | 未覆盖 |
| 包外 start.ps3 | 4726 | 330 | 320 | 1 | 30 | KOKORO, KYOKO, AI, HOTARU1, HOTARU2, OP |

## 未在本次路线中服务的系统命令

以下是静态出现但本次未服务的 ID，不等同于未实现；已服务的命令也可能仍有未实现的参数/模式。

`0x2001`, `0x2003`, `0x2008`, `0x2034`, `0x2035`, `0x203e`, `0x2065`, `0x2069`, `0x206c`, `0x2078`, `0x208c`, `0x2099`, `0x20aa`, `0x20af`, `0x20c8`, `0x20ca`, `0x20d0`, `0x20f1`, `0x20f3`, `0x20fc`, `0x2106`, `0x2131`, `0x2132`, `0x2133`, `0x2135`, `0x213d`, `0x2140`, `0x214c`, `0x2154`, `0x2157`, `0x2158`, `0x2159`, `0x2163`, `0x2164`, `0x216b`, `0x216c`, `0x216d`, `0x216e`, `0x216f`, `0x219a`, `0x219c`, `0x219d`, `0x219e`, `0x219f`, `0x21a3`, `0x21a9`, `0x21ab`, `0x21ad`, `0x21af`, `0x21e3`, `0x21e4`, `0x220c`, `0x221a`, `0x221b`, `0x2227`, `0x2229`, `0x222a`, `0x222b`, `0x2234`, `0x2235`, `0x2238`, `0x2239`, `0x223a`, `0x223b`, `0x223c`, `0x223d`, `0x2260`, `0x2261`, `0x2262`, `0x2263`, `0x2264`, `0x2287`, `0x2289`, `0x228c`, `0x228e`, `0x22a0`, `0x22a2`, `0x22a8`, `0x22ae`, `0x22b5`, `0x22b6`, `0x22b7`, `0x22b8`, `0x22ba`, `0x22bb`, `0x22bd`, `0x22c4`, `0x22c5`, `0x22c8`, `0x22c9`, `0x22d3`, `0x22d4`, `0x22d5`, `0x22d6`, `0x22d7`, `0x22d8`, `0x22d9`, `0x22da`, `0x22db`, `0x22dc`, `0x22dd`, `0x22de`, `0x22df`, `0x22e0`, `0x22e1`, `0x22e4`, `0x22e7`, `0x22e8`, `0x22e9`, `0x22f8`, `0x22fc`, `0x22fd`, `0x230b`, `0x230c`, `0x2321`, `0x2322`, `0x2328`, `0x2329`, `0x2344`, `0x2346`, `0x234a`, `0x234f`, `0x235c`, `0x2360`, `0x2365`, `0x2366`, `0x2367`, `0x2368`, `0x2369`, `0x2370`, `0x2371`, `0x2372`, `0x2373`, `0x2374`, `0x2375`, `0x2376`, `0x2377`, `0x237c`, `0x237d`, `0x237e`, `0x2380`, `0x2382`, `0x2383`, `0x2385`, `0x2386`, `0x2387`, `0x2389`, `0x238b`, `0x238c`, `0x238e`, `0x238f`, `0x2390`, `0x2392`, `0x2393`, `0x2396`, `0x2397`, `0x2398`, `0x239a`, `0x239b`, `0x239d`, `0x239e`, `0x239f`, `0x23a0`

## 证据身份

游戏数据、PC 日志、重放探针与输出保留在忽略目录或游戏目录，Git 只保留元数据报告。

| 输入 | SHA-256 |
|---|---|
| `local/recheck-20261031/structure.tsv` | `8fac5f6b980ebfef96e4eb61c3414674f58537f1201536978b75a22b59f20f41` |
| `local/recheck-20261031/replay.rs` | `f156a04ac96f40158a8d9fea8d5a26956de63b2d84ee5389c47d262e3952a13a` |
| `local/recheck-20261031/replay.exe` | `1bb9805908a57766814a7461deb02cd3a8f69ccfe0227416453d57a1f3366bd8` |
| `local/recheck-20261031/replay-long.rs` | `6c39193cc395c07d242145de5d96b3e4a9139bed98a04a08de20522d824ceab7` |
| `local/recheck-20261031/replay-long.exe` | `ca11e4b3e3ae16e7b096468d2feed8137ff538526e59f9f47c1cac9108222f92` |
| `local/recheck-20261031/gallery.rs` | `33ba769d08d3f370f9352a42658ae276c21d71037962dac562751fc9710970cc` |
| `local/recheck-20261031/gallery-final.exe` | `b749582b04a7795d963b364fc19ce3ec27e9f54da99a45b3426bc81edea4260b` |
| `local/script_coverage.py` | `e84b8944705183adfcb7d722af741bda29a24a72922c3e6a5e5c798982a9907d` |
| `E:/Download/AMATSUTSUMI/cmvs64.exe` | `dc74d2001387ae1aefbdc4204216de5ebb9d3896009280720c4e7a91b42d84f5` |
| `E:/Download/AMATSUTSUMI/data/pack/script.cpz` | `92b367e8c3d2791f1dd2ca8426581974fbb640d2766f2e82ab12383d2f5359e4` |
| `E:/Download/AMATSUTSUMI/data/pack/start.ps3` | `c338313949a803e834acf0d746c93f09c4d41f558af8fb6c13ab1ee55aff2bd0` |
| `local/recheck-20261031/kokoro.log` | `36bade0c7b9f2a82fb6cb1729c9031bfc408cd327d720555988531d097ad7718` |
| `local/audit-recheck20261031-kokoro-pcs.tsv` | `050f76f0f0ba89cd56c5f76dfa65b5b8a84796f287300d3b9d84368fcd446df8` |
| `local/recheck-20261031/kyoko.log` | `c0ca68b1b5d358eccd5f7c4b87498a478cd22dc76ce3e860446d3d442bcecef1` |
| `local/audit-recheck20261031-kyoko-pcs.tsv` | `28336b1ec3ea66c9a480aeb97440c20135cd51badd527c9aa7f460173416e14b` |
| `local/recheck-20261031/ai-final.log` | `f1ca33126e7ce33881130e9f33ae8b17e2b7bdd98b626106c5782e19d44402df` |
| `local/audit-recheck20261031-ai-final-pcs.tsv` | `a459f5a615139ebdcee4c722e8d64a7dc36a111c05e0eb6c9e90485d4be4c5e3` |
| `local/recheck-20261031/hotaru1-final.log` | `45b42c82132183e875136cbc7530d68f657cd872d8e77c34d26fac6e24bfad45` |
| `local/audit-recheck20261031-hotaru1-final-pcs.tsv` | `d6e1beb6b128a470d17ac992f227c3ea7f421d03cde18faa656ae74100f18dc4` |
| `local/recheck-20261031/hotaru2-final.log` | `d06a9aeb9e1ece7806fb13d74a1cf50fdc36c32a22165f502efca5828ce1de4c` |
| `local/audit-recheck20261031-hotaru2-final-pcs.tsv` | `b539c5ae911d29565db219385a133381db76c1d206eeb271e425f80e15d6f82f` |
| `local/recheck-20261031/op.log` | `9efece17b788eebc56d3fad7fc5b285a767fca511329759bd34d86a46fee21af` |
| `local/audit-recheck20261031-op-pcs.tsv` | `96532e03234567e57635a686f92441f5ef9a0a3cd85efeefdacb0d055298b3e2` |
