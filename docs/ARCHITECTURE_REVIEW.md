# Orion 架构与代码结构 Review

审查日期：2026-09-21。基线：`9144450`（工作区起初干净）。

后续已按本报告实施模块重构，当前结构、约束与验证见 [当前代码结构](ARCHITECTURE.md)。下文的文件位置和行号保留为审查基线的证据。

范围：现有 Rust workspace、React/Tauri 客户端、测试、benchmark 的职责边界、当前产品/API/技术文档，以及全部 11 个提交（含 2 个 merge）。仓库共有 20 个 Rust/TS/TSX/CSS 文件、4,390 行，包含测试和 benchmark。以下是当前代码的结构审查与优化建议，不代表已实施的架构变更。

## 总体判断

顶层方向合理：`orion-core` 负责扫描与索引，`orion-server` 统一持有任务并提供 HTTP API，Tauri 管理桌面与后台进程，React 负责交互。核心库没有依赖 Axum/Tauri，客户端没有复制扫描逻辑；这些边界应保留。

当前最值得投入的是模块内部的职责和状态边界。主要问题并非总行数，而是不同功能反复修改同一处状态编排、锁和数据模型。主题、导航修复、布局调整、后台生命周期、分页等提交都修改了 `App.tsx`；后台生命周期提交又把任务退出规则继续加入 Server 的 `lib.rs`。

建议按模块逐步收敛。当前规模足以用现有 crate、普通 Rust 模块、React hook/reducer 解决大部分问题。

## 1. 优先：让前端异步结果经过统一的状态入口

**证据**：`apps/desktop/src/App.tsx:65` 的轮询、`:95` 的目录查询、`:111` 的详情查询、`:147` 的启动扫描、`:169` 的取消操作，各自更新状态。任务切换、导航、翻页和调整页大小分别维护相似的清空规则（`:90`、`:120`、`:131`、`:140`）。单个 `App` 同时管理连接、请求身份、扫描、浏览位置、原生目录选择、复制和整页 JSX。

**已复现的问题**：旧任务 A 的 `/tasks` 请求尚未返回时启动任务 B，`startScan` 先把界面切到 B；随后 A 的旧轮询响应在 `:79` 无条件 `setTask`，界面又切回 A，直到下一次成功轮询。临时诊断使用延迟的模拟响应，观察到文件数由 B 的 987 回退成 A 的 201。现有查询的 `AbortController` 保护了目录/详情切换，但没有把轮询与启动/取消放进同一个时序规则。

**建议边界**：

- `useServerSession`：连接、实例身份、在线状态、重连、轮询；连接切换递增 session generation。
- `useScanTask`：当前任务、启动/取消、幂等 request ID、mutation generation；统一接纳和拒绝异步响应。
- `useExplorer`：`taskId / parent / offset / pageSize / selected`，用 reducer 集中表达任务切换、导航和分页转换。
- `ConnectionPanel`、`ScanControls`、`ScanSummary`、`DirectoryExplorer`、`EntryDetails`：展示及局部交互。
- 一个小的 desktop adapter 隔离 `invoke/open/revealItemInDir`，让功能 hook 不必知道运行于浏览器还是 Tauri。

轮询结果至少要满足当前连接和操作 generation；同一任务的旧 revision 不得覆盖已接纳的新 revision。任务 ID 改变仍需接受其他客户端发起的新任务，不能简单永久拒绝与当前 ID 不同的结果。请求超时后的幂等 ID 必须继续保留。

**验收**：旧轮询晚于 start/cancel 返回、切换连接时旧请求返回、任务被其他客户端替换、快速导航与翻页。原有重复点击根目录、分页、迟到详情测试继续通过。

## 2. 优先：抽出任务协调模块，缩短全局任务锁的持有时间

**证据**：`crates/orion-server/src/lib.rs:114` 的 `AppState::start` 在 `:115` 取得 `Mutex<Tasks>`，到 `:163` 调用 `Scan::new` 时仍持锁；`crates/orion-core/src/lib.rs:199` 的构造函数执行 `symlink_metadata` 和 `canonicalize`。替换 `tasks.latest`（Server `:189`）也可能在锁内释放旧的大型索引。

`start` 已经通过 `spawn_blocking` 执行，这个方向正确；但 `/tasks`、任务查找和 shutdown 仍从 async handler 同步争用同一把锁（Server `:51`、`:97`、`:331`）。当根目录校验遇到慢盘或不可及时响应的路径时，其他请求可能等待任务锁，并占住 Tokio worker。这里是代码可确认的阻塞路径；本次没有实测慢盘下的延迟。

同一个 `AppState` 还承载 HTTP token、实例 ID、任务保留策略、幂等记录、线程启动和退出通知，应用规则直接返回带 `StatusCode` 的 `ApiError`。后续增加任务种类或保留策略时，HTTP 层和应用编排会一起增长。

**建议**：

- 在现有 Server crate 中引入 `TaskCoordinator`，负责单任务规则、幂等、启动/取消/退出；返回独立的 `StartError/TaskError` 等应用错误。
- `http` 模块只负责参数解析、认证、路由，以及应用错误到 HTTP 状态的映射。
- 用短临界区登记待启动请求及其身份，再在锁外做文件系统校验，最后按预留身份提交结果；shutdown 必须能阻止已经登记但尚未提交的扫描开始运行。
- 相同 request ID 的并发请求应关联同一份待启动结果。校验失败、线程启动失败和退出竞争都应释放预留状态。
- 旧索引用 `take/replace` 移出锁后释放；查询先克隆 `Arc<Scan>` 再释放任务锁。

不要仅把 `Scan::new` 移到锁前：那会削弱现有幂等和单任务保证，也会让退出检查与新扫描启动之间出现竞争。内部 pending 状态可以先不变更对外 API。

**验收**：用可控制完成时机的根目录校验替身验证：校验暂停时任务查询仍返回；退出与启动竞争不会漏出新 worker；并发相同 request ID 只启动一次。现有生命周期和幂等测试保留。

## 3. 优先：给目录查询建立独立的读取与缓存边界

**证据**：`crates/orion-core/src/lib.rs:457` 的 `Scan::list` 在读锁内复制目录的全部子项，并为名称创建 String（`:473`）；释放锁后排序整个集合（`:480`），最后才 `skip/take`（`:492`）。无论客户端取 15、50 还是 100 项，计算量都由目录总条目数 N 决定：复制和临时内存为 O(N)，排序为 O(N log N)。排序已在锁外，值得保留；锁内的全量复制仍会与扫描写入竞争。

前端目录查询依赖整次扫描的 `task.revision`（`App.tsx:109`）。其他目录更新也会使当前目录重查。当前 benchmark 的计时只覆盖构造、扫描和 summary（`crates/orion-core/benches/scan.rs:282`），明确不包含列表排序及 GUI 轮询，因此不能用现有扫描加速结果证明分页查询已经足够快。

**建议按成本递进**：

1. 先为已结束的扫描缓存访问过的目录排序结果，缓存条目 ID；翻页从稳定排序中切片，仅构造当前页的视图。
2. 对运行中的扫描，按目录维护直接子项列表/排序依据的变更版本；子目录大小变更时，父目录的排序缓存也必须失效。对外全局 revision 语义暂时保留。
3. 每次返回的条目仍须来自同一快照。不能锁外保存 ID，随后跨多个索引版本逐个读取而拼出混合响应。
4. 只缓存实际访问的目录并限制容量，避免给整棵树建立第二份完整对象副本。

**验收**：宽目录的首查和翻页时间、扫描同时查询时的 p50/p95、取消响应、峰值内存、扫描总耗时。可从 1 万和 10 万直接子项开始，并与实际目录交叉验证；本次未运行新的性能测试，也不预估加速倍数。

## 4. 随查询优化进行：把扫描调度和索引写入真正分开

**证据**：Core `lib.rs` 同时包含领域类型、索引表示、任务状态、路径检查、查询和历史串行实现。`parallel.rs:1` 直接使用父模块的 `Entry`，`:260` 的 `publish_batch` 直接取得 `Scan.index` 并修改 entries、祖先汇总、错误和 revision。当前已按“并行代码”拆文件，但扫描器依旧掌握索引的内部布局。

影响是增加排序缓存、目录级版本或新的枚举方式时，worker 也需要了解这些索引不变量；历史 benchmark 实现又在同一个文件中维护另一条写入路径。

**建议保持 `Scan` 作为对 Server 的稳定 facade，在 Core 内部划分**：

```text
orion-core/src/
  lib.rs             对外导出
  model.rs           状态、条目及查询结果类型
  scan.rs            Scan 生命周期、取消、对外接口
  index.rs           存储、批量提交、汇总和版本不变量
  query.rs           列表、详情、排序缓存
  scanner/
    mod.rs           发现批次的内部数据结构
    parallel.rs      队列、worker 和工作完成
    filesystem.rs    目录读取、metadata、链接与范围检查
  legacy_scan.rs     受 feature/test 限制的历史 benchmark 对照
```

关键是 `Index::apply_batch(...)` 统一维护索引不变量，scanner 提交发现结果并取得待扫描目录 ID；仅移动函数到不同文件不能完成这个边界。文件系统与索引之间先用具体内部类型连接，有第二种实现或需要故障注入的接缝再引入 trait。

保留已实现的约束：文件系统 I/O 不持索引锁、写索引后再入工作队列、所有 worker 退出后才进入终态、取消保留部分结果、错误采样有界且稳定。历史对照的行为与计时口径也要保持，避免重构同时改变 benchmark 基线。

## 5. 中优先级：把后台生命周期变成可正常复用的模块

**证据**：`apps/desktop/src-tauri/src/backend.rs` 同时管理连接文件、HTTP、进程、日志、会话状态及中文托盘文案。`ensure`（`:138`）、`status`（`:235`）、`shutdown`（`:267`）都在持有 session mutex 时执行阻塞网络操作；启动/退出还包含最长约 10 秒的重试循环。后台状态查询会与启动/退出串行等待。

更明确的结构信号在 `crates/orion-server/tests/lifecycle.rs:3`：通过 `#[path = "../../../apps/desktop/src-tauri/src/backend.rs"]` 编译桌面源码。这使 Server 测试依赖桌面文件位置及其依赖细节，还把 backend 内的 2 个单元测试再次运行。生产依赖并未倒置，但测试的复用方式已经跨越 package 边界。

**建议分两步**：

- 先在 desktop 内拆分 `supervisor`（启动/连接/停止状态）、`discovery`、`http_client`，由托盘模块把结构化状态转换成中文文案。状态轮询克隆连接后释放 mutex，再发 HTTP；结果提交时核对会话身份。
- 生命周期代码继续被独立进程测试复用时，提取一个小型、无 Tauri 依赖的 `orion-runtime` crate，desktop 正常依赖它，Server 的集成测试通过 dev-dependency 使用它，从而去掉源码路径包含。它只负责本地发现、连接和进程生命周期，不依赖扫描核心。

`Starting/Connected/Stopping` 等内部状态比 `connection + child + stopping` 的自由组合更容易表达合法转换。但启动和退出仍需序列化；不能为缩锁而允许同时启动两个子进程。外部后台的退出语义继续遵循当前产品约定。

## 6. 中低优先级：建立可检查的跨语言契约

**证据**：扫描响应来自 Core 的 Serialize 类型，Server 的 health/discovery 使用 `serde_json::json!`，Tauri 单独声明 `Connection/Health/Task`（`backend.rs:12`），React 又手写 DTO（`src/lib/api.ts:1`）。`Api.request` 在 `:51` 用类型断言接纳 JSON，前端测试主要模拟 Api 方法，不能自动发现真实响应与 TS 类型之间的变化。

不同客户端只声明自己需要的字段本身没有问题；例如 TS Health 比 Rust Health 少 `capabilities` 并非当前故障。风险在于状态枚举、可空字段、错误码和能力协商没有共同的变更检查，加入 CLI/MCP 后维护面会进一步增加。

**建议**：先把 Server 侧 wire DTO 和错误映射集中到 `http/contracts.rs`，以真实序列化响应样本建立 Rust/TS 契约检查。为 health、任务状态和错误码明确兼容规则，客户端对未知状态给出显式 fallback。需要新增客户端或频繁改字段时，再引入生成的 TS 类型/Schema；出现第二个需要共享完整 DTO 的 Rust consumer 时，再评估很小的 protocol crate。

Core 的领域类型带 Serialize 在当前规模下可接受。只有 wire 兼容要求开始妨碍内部模型演进时，才增加领域模型到 DTO 的映射，避免每个对象立即维护两套同形结构。

## 建议实施顺序与验收门槛

| 批次 | 内容 | 完成门槛 |
| --- | --- | --- |
| 1 | 提取前端 session/task/explorer 状态逻辑，修复已复现的旧响应覆盖 | 时序回归测试和现有导航测试通过；页面行为保持 |
| 2 | Server 拆 HTTP 与 TaskCoordinator，收缩锁范围 | 幂等、并发启动、退出竞争、慢校验下查询响应测试通过 |
| 3 | Core 提取 index/query 边界，增加目录排序缓存 | 结果/分页版本一致；新增查询负载基准证明收益且内存可控 |
| 4 | 拆桌面 supervisor，去掉测试中的跨目录源码包含 | 现有隔离进程生命周期测试通过；托盘交互单独验收 |
| 5 | 固化 API 契约检查，再按实际客户端需求生成类型 | Rust 响应变化能让 TS 契约检查失败 |

每个批次可进一步分成行为保持的移动和行为变化两个提交，便于 review 与回退。暂不需要跨进程拆更多服务、通用任务调度框架、事件总线或插件运行时。分页和响应问题也不构成立即替换 HTTP、上数据库或换前端状态库的依据。

后续做持久化时，应在已经独立的索引/查询边界讨论结果快照；做清理时，应增加独立的计划、授权与执行能力。它们都不宜继续加入现有 `Scan` 对象，但本轮无需预建。

## 验证与审查限制

- `cargo test --locked --offline -p orion-core -p orion-server -p orion-desktop`：22 次测试执行通过（包含由源码路径复用导致的 2 次重复单测）。
- 前端 Vitest：现有 3 个文件、20 项测试通过；`tsc --noEmit` 通过。
- 临时前端时序诊断：1 项通过，确认旧轮询覆盖新任务的行为；该临时文件已移除，业务代码未修改。
- 当前环境默认 npm 入口解析失败，直接调用已安装的本地 Vitest/TypeScript 完成了同等检查；Vite 读取父目录遭遇沙箱限制后，在获准的沙箱外执行成功。这些是执行环境现象，未归类为仓库缺陷。
- 本次为架构审查，未重新执行整盘 benchmark、未测列表延迟/峰值内存，也未进行原生托盘交互验收。性能项依据代码路径和现有 benchmark 的覆盖范围提出，不代表已观测到具体延迟或加速比。
- 仓库已有行为测试、隔离生命周期测试和可追溯 benchmark，是重构的良好基础。当前未发现已提交的 CI 配置；如果后续采用多人或更多分支开发，可将现有检查固化到 Windows CI，作为边界维护措施。
