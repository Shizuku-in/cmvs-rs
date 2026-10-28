# 运行指南

命令从项目根目录执行，需要 Rust 1.89+ stable。`--offline` 要求依赖已缓存。以下桌面参数组合目前以[《アマツツミ》PC 样本](../games/amatsutsumi/README.md)验证，游戏路径和字体路径请替换为本机路径。

## 桌面预览

```powershell
cargo run --offline --release --features desktop --bin cmvs-desktop -- "E:/Download/AMATSUTSUMI" --encoding shift-jis --continue-without-snapshot --skip-auto-resume --audio record --font "C:/Windows/Fonts/msgothic.ttc"
```

该命令显式绕过两项研究屏障以进入标题。它不提供完整剧情存档恢复：

| 选项 | 含义与限制 |
|---|---|
| `--encoding shift-jis` | 显式选择文本编码；另可选 `gbk`、`utf-8`，不能据此推断对应游戏已兼容 |
| `--continue-without-snapshot` | 应用已核对的实时快照影响并继续，镜像标记不可用；默认会停在快照屏障 |
| `--skip-auto-resume` | 本作首次系统存档读取成功后清零内存整数 2198，绕过自动恢复；只接受已核对游戏 ID，磁盘存档保持不变 |
| `--font <字体>` | 用桌面 OpenType 后端显示文字；字体路径由用户提供，与原版 GDI 有像素差异 |
| `--audio record` | 仅记录音频请求，不发声 |

未指定字体且未启用 `--record-text` 时保留文字输出屏障；`--record-text` 只记录字形。关闭窗口可退出；活动游戏中的 Esc 按游戏动作绑定处理。`--exit-after <秒>` 可做有界运行，`--save-frame <新文件.png>` 可导出退出时的帧。

## 原生音频

```powershell
cargo run --offline --release --features desktop,audio-output --bin cmvs-desktop -- "E:/Download/AMATSUTSUMI" --encoding shift-jis --continue-without-snapshot --skip-auto-resume --audio native --font "C:/Windows/Fonts/msgothic.ttc"
```

普通 Windows 桌面权限下已验证品牌音效和标题 BGM；受限沙箱中相同构建曾以 `0xc0000008` 退出。设备检查、源消费时钟与验证边界见[原生音频](../backends/audio-output.md)。其他平台和完整剧情音轨仍需独立实测。

## 独立系统存档目录

如需保存通关持久状态，在上述命令末尾增加：

```powershell
--user-data-dir "local/amatsutsumi/user-data"
```

目录必须位于原游戏根之外。用户 `system.dat` 优先，缺失时才从原游戏只读引入 CSS1 模板；支持重复保存、`system.bak` 与会话锁。损坏文件明确报错，不自动恢复备份。剧情槽扫描也使用此目录，但完整剧情恢复仍未实现；无 CSS1 模板的首次保存尚不支持。

单次实验可改用 `--system-save-output "local/amatsutsumi/completed-system.dat"`，只允许游戏目录之外的新文件、拒绝覆盖。该选项与 `--user-data-dir` 互斥。不配置这两种方式时保留 `0x2129` 系统存档写入屏障。细节见 [CSS1](../formats/system-save-format.md)。

## 研究与检查

脚本 trace、资源检查和导出示例见 [CLI 指南](cli.md)。桌面、输入和字体的具体支持分支见[后端文档](../README.md)，当前结局路线与鉴赏停点见[游戏档案](../games/amatsutsumi/README.md)。原始文件保持只读，真实输出放游戏目录外并避免进入 Git。
