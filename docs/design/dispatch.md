# 系统命令分派层设计草案

> 状态：**草案；M1 已实现**（2026-10-31，基线 `da26a22`，结果见第 9 节）。M2 起的接口在实施前再确认。档案、证据状态与闸门的来历见[引擎档案层设计](engine-profiles.md)，本文不重复。

2026-11-01增量试点：Aoitori接入暴露真实文字布局差异，先迁移涉及的文字切片。`CommandGate::resolve`先执行闸门，再返回`Root`或显式研究语义及首次放行通知；`dispatch::ROOT`登记6个根入口，`FONT_CELLS`有8个稀疏研究覆盖。runner/trace共享`run_text`，选中覆盖后不能退回根；分派入口不读取构建ID，宿主不接触档案。新增处理器使用`Call`和`Flow`，公共命令包装只共享不变流程，差异算法独立。证据仍为Unreviewed，必须研究放行；这不是正式M4的可默认放行Variant，也不表示全部M2/M3完成。其余责任链与完整归属审计继续按本文推进，详见[Aoitori记录](../games/aoitori/research.md)。

## 1. 要解决的问题

档案层回答了"这个构建的命令能不能套用基准语义"，但"语义对应的 Rust 代码在哪"仍由责任链决定：

- `runner::Startup::step_once` 依次尝试约 60 个 `try_*` 调用，`cmvs-trace` 另有约 30 处自己的链。命令归哪个宿主处理，只能靠搜索；同一个宿主内部又是一长串 `if matches!(command, …)`。
- 优先级由调用顺序隐式决定。新宿主排在哪里，可能改变其他命令的行为。
- runner 和 trace 两条链重复，[档案设计 6.3](engine-profiles.md#63-分派闸门) 已经指出漂移风险，闸门只是共用了检查函数，分派本身仍未合并。
- [档案设计 6.4](engine-profiles.md#64-变体) 原计划在第一个 `Variant` 出现时给宿主的 `try_handle` 加档案参数。这会让版本判断分散到各个宿主里。
- 处理函数用 `vm.stack().peek_u32(4)` 这类偏移取参数，参数含义只在注释里，参数个数错了也不会报错。
- 返回值有 `bool`、`Option<Step>`、`StartupEvent` 三种形式，等待重试的约定散落在各宿主中。

## 2. 目标与非目标

**目标**

1. 每层只回答一个问题：证据层决定用哪套语义，分派层决定代码在哪，处理层决定做什么。
2. opcode 到处理函数的归属一处可查，并由测试保证唯一。
3. runner 和 trace 走同一个分派入口。
4. 变体只在分派表中替换处理函数，宿主不知道档案和版本。
5. 迁移期间《アマツツミ》trace 逐字节一致。

**非目标**

- 不为每个版本写一套 trait 实现。用默认方法"继承"语义，等于无证据继承。
- 不为每个构建复制整张表。19 个构建各 700 多项，几乎全部相同。
- 不在运行时动态注册处理函数；表是静态的，正确性由测试保证。
- 不按版本拆 crate，不改变现有命令语义。

## 3. 分层

```text
游戏数据 ──识别──▶ ProfileSelection                          （已有）
VM Step::System { pc, op }
   │
   ▼ 证据层  profile.resolve(op) → Semantics::Root | Semantics::Variant(id) | GateError
   ▼ 分派层  ROOT[op] 或 VARIANTS[(id, op)] → Entry { handler, name, root_address }
   ▼ 处理层  handler(&mut Hosts, &mut Call) → Flow
   ▼ 后端    渲染 / 音频 / 平台 / 存储 trait                  （已有，不动）
```

## 4. 证据层

### 4.1 状态

在档案设计 4.3 的基础上补齐两种状态：

| 状态 | 证据 | 解析结果 |
|---|---|---|
| `Base` | 根档案本身 | `Root` |
| `Inherited` | 同架构完整指纹，与两边 exe 哈希绑定 | `Root` |
| `Reviewed` | 人工复核记录，见 4.2 | `Root` |
| `Presumed` | 可比较部分的形状指纹相同，仅被保守边界挡住，见[档案设计第 15 节](engine-profiles.md#15-指纹策略-v3-与-presumed2026-10-31) | `Root`，单独计数并在停点列出 |
| `Variant(id)` | 人工复核记录，确认与根语义不同 | `Variant(id)` |
| `Unreviewed` | 无 | 默认报错；`--allow-unreviewed` 时按 `Root` 放行并记录 |
| `NoOp`、`Absent` | switch 结构 | 总是报错 |

### 4.2 复核记录

`Reviewed` 行在 case RVA 之后给出复核文档，路径相对于 `docs/`：

```text
0x201f reviewed 0x7ecd9 0x70d92 review=games/chronoclock/vm-semantics.md
```

- 文档必须存在，且包含以该命令号（如 `0x201f`）为标题的小节。小节写明两个构建的处理函数地址、所对应的 exe SHA-256、比较范围和结论；地址与哈希的绑定由文档承担，档案行只做引用。
- 解析时文档内容不参与校验（核心不读 `docs/`）；由仓库测试检查路径与小节存在。
- 复核只在同架构的根构建与派生构建之间进行，与指纹工具的约束一致。
- `Variant` 的格式在出现第一个真实变体时确定（M4），预期为 `variant=<id> review=<doc>`。

### 4.3 解析接口

闸门从"检查是否放行"改为"解析语义来源"：

```rust
pub enum Semantics { Root, Variant(VariantId) }
impl CommandGate<'_> {
    pub fn resolve(&mut self, vm: &Machine<'_>, pc: usize, op: u16)
        -> Result<Semantics, GateError>;
}
```

计数按证据来源分别统计 `Base`、`Inherited`、`Reviewed`、`Unreviewed`。将来引入时代锚点时，档案 `base` 可指向锚点，形成"根 → 锚点 → 派生"的链；链如何解析只由 `resolve` 处理。

### 4.4 复核的工作量

CHRONOCLOCK x64 比较报告中，325 条未复核命令里只有 10 条是"完整指纹不同"。其余都是工具的保守边界：可写或未解析的数据引用 279 条、间接控制流 183 条、深度截断 170 条、递归环 161 条、未解码指令 133 条（一条命令可有多个原因）。因此复核分两路：沿默认 trace 的停点逐条人工复核；同时改进导出工具，在不降低保守性的前提下减少误报。指纹策略 v2 已把运行库函数作为叶子比较，未复核数降到 277 条，见[档案设计第 14 节](engine-profiles.md#14-指纹策略-v22026-10-31)。v3 再建模 switch 与映像相对访问并引入 `Presumed`，剩余 88 条未复核命令都有可见差异（完整指纹不同 11 条、形状不同 78 条，其中 1 条已人工复核）。

### 4.5 跨构建复用（待第二款游戏）

目前每个派生档案只与根档案比较。接入第二个同时代构建时，比较对象扩展为所有已建档的同家族、同架构构建：新构建的某条命令若与已建档构建 R 的同号命令完整指纹相同，而 R 中该命令为 `Inherited` 或 `Reviewed`，新构建就沿用这条证据，档案记录证据链（经由哪个构建、哪份证明或复核）。形状指纹相同只能传递 `Presumed`，不能升级为继承。相邻年份的构建通常比它们与根更接近，人工复核因此由多款游戏分摊。实现要放开 `Registry` 现在"派生档案只能以根为基准"的限制，并给证明格式加上参照构建；在只有一个派生档案时这些接口无法用真实数据检验，所以推迟到接入第二款游戏时实现。

## 5. 分派层

新增 `cmvs-core::dispatch`：

```rust
pub struct Entry {
    pub name: &'static str,
    pub root_address: u64,               // 根档案 x64 处理函数入口
    pub handler: fn(&mut Hosts, &mut Call) -> Result<Flow, DispatchError>,
}
pub static ROOT: [Option<Entry>; 0x400] = …;
pub static VARIANTS: &[(VariantId, u16, Entry)] = &[];
```

- `ROOT` 以 `op - 0x2000` 为下标；变体表是稀疏覆盖。普查显示命令号单调增长、没有重编号，预期变体很少。
- 未登记的项按现有方式报 `Unhandled`。
- 测试保证：每个 opcode 至多一个处理者；档案引用的每个变体在表中存在；`cmvs-profile` 可输出覆盖矩阵（有证据且已实现、有证据未实现、已实现但所有档案都不放行）。
- runner 和 trace 都调用 `dispatch::run`。trace 特有的记录行为放在后端或 `Hosts` 的记录实现中，不另建分派链。

## 6. 处理层

- **`Call`**：携带 `pc`、`op` 和栈视图。`pop_u32("slot")` 等按名字取参数，错误信息带参数名；`require_consumed()` 检查参数个数；`complete(SystemResult)` 结束命令。
- **`Flow`**：`Done`、`Wait`（保留待处理命令，下一帧重试）、`Event(StartupEvent)`，取代现有三种返回形式。
- **`Hosts`**：持有全部宿主。处理函数按字段借用所需宿主，`try_handle_input(machine, scene, input)` 这类跨宿主调用有统一写法。
- 宿主不接触档案。版本差异只体现为分派层选择了哪个处理函数。

## 7. 风险与待确认

1. **条件认领**：部分宿主根据运行状态决定是否接手同一命令（如拖动与手势）。M2 的归属检查先找出所有重叠，迁移时把条件判断收进单个处理函数。
2. **顺序依赖**：责任链中存在"先到先得"的隐式优先级。合并前逐项确认，回归以《アマツツミ》逐字节 trace 为准。
3. **runner 与 trace 的差异**：两条链现有的行为差异需要列表核对，不能在合并时悄悄消失。
4. **返回值语义**：`Wait` 必须与现有的等待和恢复（含存档恢复）行为完全一致。

## 8. 分阶段

| 阶段 | 内容 | 验收 |
|---|---|---|
| M1 | 加入 `Reviewed` 与复核记录格式；复核 CHRONOCLOCK 默认停点 `0x201f` | 默认 trace 停点前移；复核记录与地址、哈希绑定；《アマツツミ》不受影响 |
| M2 | 只读归属表与重叠检测；新写和改动的处理函数使用 `Call` | 运行时行为不变 |
| M3 | 统一分派表，runner 与 trace 合并；`resolve` 取代 `check` | 《アマツツミ》trace 逐字节一致 |
| M4 | 第一个 `Variant` | 照常走语义证据流程 |
| M5 | 封包代际、时代锚点 | 另行设计 |

M3 应在 M4 之前完成，第一个变体就不需要修改宿主签名。

## 9. M1 实施结果（2026-10-31）

基线 `0fc79d9`。新增 `CommandStatus::Reviewed` 与 4.2 的 `review=` 记录：解析要求路径为 `docs/` 下的相对 `.md`、不含 `..`；只能出现在派生档案的处理函数行，根档案对应命令必须是 `Base`。仓库测试检查每条记录指向的文档存在，且有以该命令号为标题的小节。闸门把 `Reviewed` 当作根语义放行，`GateCounts` 增加 `reviewed`；`cmvs-profile identify/show` 与 trace 统计行同步列出。比较工具重新生成候选档案时原样保留 `reviewed` 行，不再整体拒绝。

| 验收项 | 结果 |
|---|---|
| 复核记录 | CHRONOCLOCK `0x201f` 登记为 `Reviewed`，证据见[VM 语义](../games/chronoclock/vm-semantics.md)；处理函数两边相同，读取方只在取值 1、2 上不同，Rust 不解释该值 |
| CHRONOCLOCK 默认 trace | 停点由 `start.ps3 / 0x1a / 0x201f` 前移到 `start.ps3 / 0x50 / 0x2091`；统计 inherited=3、reviewed=1；与 P2 日志的共同前缀逐字节一致 |
| 《アマツツミ》回归 | 与 P2 同参数百万步 trace，去掉 P2 日志的 CR 后逐字节一致（1220073 行），均以预算耗尽退出码 1 |
| 检查 | `cargo fmt --check`；默认离线测试 836 项通过（12 项忽略）；严格 Clippy；Python 合成测试 11 项 |
