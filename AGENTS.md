# cmvs-rs 开发约定

- 项目名 cmvs-rs，目标用 Rust 实现 CMVS 全平台支持，准确说明当前完成度。
- 开发前阅读 README.md 及 docs/ 下的 research、architecture、references。
- 平台无关核心优先，系统接口留在后端，不新增 C++ 运行核心。
- 原游戏只读，素材、解包剧本、IDA 数据库不进入 Git。
- 提交前运行 cargo fmt --check、cargo test --offline、cargo clippy --offline --all-targets -- -D warnings。
- 改动解码算法后使用外部真实样本与 Python 对照；测试仅含合成数据。
- 记录参考项目提交和许可，不直接复制未明确授权的源码。
- 指令语义保留证据，未知指令显式报错，地址与二进制哈希绑定。
