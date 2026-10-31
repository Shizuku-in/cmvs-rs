# cmvs-rs 开发约定

- 项目名 cmvs-rs，目标用 Rust 实现 CMVS 全平台支持，准确说明当前完成度。
- 开发前阅读 README.md 及 docs/ 下的 research、architecture、references。
- 文档入口为 docs/README.md；涉及具体游戏时另读 docs/games/<游戏标识>/README.md 及对应研究/语义证据。通用格式与后端说明分别放 docs/formats/、docs/backends/，游戏实测结果按游戏归档，不向项目 README 追加阶段流水。
- 平台无关核心优先，系统接口留在后端，不新增 C++ 运行核心。
- 原游戏只读，素材、解包剧本、IDA 数据库不进入 Git。
- 提交前运行 cargo fmt --check、cargo test --offline、cargo clippy --offline --all-targets -- -D warnings。
- Commit message 格式：`<type>(<optional scope>): <description>`；scope 可省略，省略时使用 `<type>: <description>`。
- 改动解码算法后使用外部真实样本与 Python 对照；测试仅含合成数据。
- 记录参考项目提交和许可，不直接复制未明确授权的源码。
- 指令语义保留证据，未知指令显式报错，地址与二进制哈希绑定。
