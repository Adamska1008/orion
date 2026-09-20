# 扫描 benchmark

通过标准 `cargo bench` 直接运行扫描核心，不启动 Server 或 GUI，不需要 PowerShell / Node 启动脚本。用于验证扫描改动的性能及结果一致性。

2026-09-21 模块拆分后，扫描报告 schema 升为 4：`core_source_sha256` 覆盖 lib/model/index/scan/query/legacy_scan/scanner 各源码的有序、带长度拼接，`parallel_source_sha256` 指向新位置 `scanner/parallel.rs`。旧报告保留原 schema。历史串行算法保留，额外维护目录缓存版本，故新报告也不能直接作为与旧二进制的严格前后性能比较。

## 目录查询基准

```powershell
cargo bench -p orion-core --features bench-internals --bench query
```

该入口生成 1 万、10 万条目的合成内存索引（只创建临时根目录，不创建对应数量的磁盘文件），以每页 50 项、40 对交替顺序请求比较原全量复制/排序与缓存命中。每对都比较完整 JSON；首次缓存查询单独记录。随后另测批量写入期间的查询，并记录发布结束后的取消调用耗时。JSON 输出包含原始时间样本、环境和源码指纹；可重定向 stdout 保存报告。

本次报告见 [query-cache.json](benchmarks/2026-09-21/query-cache.json)：10 万条目的未缓存查询 p50 为 20.1951 ms，缓存命中 p50 为 0.0188 ms，首次查询为 22.3066 ms。变化中的目录仍可能反复未命中；并发阶段样本数较少，只作诊断，不与稳定索引的缓存命中混为一谈。缓存有 32 个目录和 100 万 ID 的容量上限，但这里没有实测进程峰值内存。该基准不包含文件系统枚举、HTTP 或渲染。

## 运行

在项目根目录执行：

```powershell
cargo bench -p orion-core --features bench-internals --bench scan -- --baseline serial --workers 4 --runs 8 --warmups 2 --label concurrency-ab --output target/bench-results/concurrency-ab.json
```

默认执行三组固定结构的数据集。数据生成在项目 `target/bench-fixtures` 下的专用临时目录，计时前完成，运行结束后只清理该临时目录。不会写入被测试的真实目录。

| 数据集 | 文件数 | 目录数（含根） | 用途 |
| --- | ---: | ---: | --- |
| `wide-v1` | 12,000 | 1 | 一个目录含大量小文件，检验逐文件额外查询的成本 |
| `tree-v1` | 4,096 | 341 | 四叉目录树，混合小文件与 4 个较大文件 |
| `deep-v1` | 512 | 33 | 32 层目录，观察路径检查与祖先汇总成本 |

文件名、目录结构与逻辑长度按固定配方生成，包含中文名称。文件用 `set_len` 设置长度；扫描不读取内容，这些样本不用于测读写带宽。可用 `--suite wide` / `tree` / `deep` 单独运行一组。相对 `--output` 路径相对于项目根目录解析。

复现历史 metadata 实验（不包含并发和批量更新）：

```powershell
cargo bench -p orion-core --features bench-internals --bench scan -- --baseline legacy --candidate enumerated --label metadata-ab --output target/bench-results/metadata-ab.json
```

另有 `--suite hardlink` 正确性样本：创建两个指向同一文件的名称，再通过其中一个名称把文件从 4 KiB 改成 8 KiB。按当前「每个路径分别计数」的口径，总计应为 16 KiB。该样本规模太小，只用于检查正确性，不用于推断性能；不包含在默认三组性能数据中。

真实目录测试：

```powershell
cargo bench -p orion-core --features bench-internals --bench scan -- --root 'C:\Users\dell\codes\orion\node_modules' --runs 8 --warmups 2 --label real-directory --output target/bench-results/real-directory.json
```

选择内容稳定的目录；关闭会修改该目录的构建、下载、同步任务，避免同时全盘扫描或运行重度磁盘任务。报告会记录真实目录路径，分享前注意其中的个人信息。当前程序不自动结束其他进程，也不调整系统缓存、权限、电源或安全软件设置。

## 比较方法

- `--baseline serial`（默认）：保留并发改动前的单线程遍历，已经使用 `DirEntry::metadata()`，但仍逐条写锁和逐文件祖先汇总。
- `--baseline legacy`：更早的单线程实现，每项调用 `fs::symlink_metadata(path)`，用于复现 metadata 实验。
- `--candidate current`（默认）：调用正式实现的 `Scan::run_with_workers()`，包括目录线程池、批量写索引与批量祖先汇总。`--workers` 为 1–16，默认与 Server 的默认配置相同；benchmark 不读取 Server 的环境变量。用 1 线程观察批量更新等改动，再与 2/4/8 线程结果比较，不把所有收益都归因于线程数。
- `--candidate enumerated`：保留的单线程枚举实现，与 `serial` 基线相同，可搭配 `--baseline serial` 做 A/A。两个旧入口共用原遍历函数，新实现在 `parallel.rs` 中；错误明细的有界稳定采样由所有入口共用。
- 每组先执行一次候选优先的正确性检查（`preflight`），再各预热 2 次、各测 8 次；预热与测量顺序交替为 `baseline/candidate`、`candidate/baseline`。`--runs` 必须是大于等于 4 的偶数。先检查候选是为了避免基线的逐路径查询刷新缓存、掩盖初始差异。
- 每次新建扫描对象。计时包含根目录校验、完整遍历、索引构建、汇总及读取摘要；不含编译、造数、完整性指纹计算、报告输出和销毁索引。
- 使用 Cargo 的优化 `bench` profile，Debug 构建会被拒绝。两个实现位于同一二进制中，使用相同依赖、编译选项、进程和数据集；旧查询入口只在 `bench-internals` feature 下暴露。
- 原始报告保存每次耗时、吞吐量、执行顺序、阶段、计数和指纹，同时记录 Rust 版本、CPU 标识、架构、Git HEAD、扫描源码、并发模块与 benchmark 源码 SHA-256。schema 3 增加基线类型和线程数，分布统计字段由 `comparison.legacy` 改为 `comparison.baseline`；旧报告保留原 schema。吞吐量为文件数加目录数除以耗时，不包含链接等跳过项。
- 汇总给出最小值、中位数、最大值及中位绝对偏差（MAD）。加速比与耗时下降百分比取每对 A/B 结果的中位数，不取最快一轮。

这是**预热后的扫描核心测试**。操作系统缓存没有清空，不能声称冷启动性能；合成目录也不能代替 C 盘实测。进度轮询、API 排序及界面渲染不在计时范围，端到端体验仍需单独试用。

## 结果一致性

合成数据必须符合配方规定的文件数、目录数和总大小，并且没有跳过或错误。每轮在计时后对条目的相对路径、类型、汇总大小、修改时间和枚举状态按路径排序并计算 SHA-256；Windows 路径用原始 UTF-16 编码，避免有损名称转换。

所有预检、预热与正式样本都必须完成扫描，且有相同的条目指纹、统计数量及错误摘要（按路径/代码稳定选择最多 100 条与总错误数）。另存不含修改时间的 `structure_digest` 帮助定位差异，但不替代完整校验。真实目录可存在稳定的已报告跳过项，但不等于完整覆盖。任一样本不同或不符合配方计数，则报告 `valid: false`、`comparison: null`，保存原始数据，并以非零退出码结束；不能把少扫或漏扫当作提速。

目录持续变化可能使对比无效，这是预期行为。基准不会自动忽略差异，也不会悄悄排除错误样本。

## 当前实验

归档的 metadata A/A 检查来自启用优化之前：当时 `current` 与 `legacy` 相同，再用 `enumerated` 测 A/B。如今默认命令比较单线程枚举和正式并发实现。小于波动范围的差异不视为可靠提升；原始 JSON 保留预检、预热及全部测量值。

并发原始数据见 [concurrency](benchmarks/2026-09-20/concurrency/)：包含真实目录 1/2/4/8 线程、合成数据及 tree 差异诊断。tree 的结构指纹一致、完整指纹不同，差异只涉及修改时间，仍标为无效且不计算加速比；没有为了并发而放宽结果校验。默认最多 4 线程是初始资源上限，不代表每种磁盘的最优值；纯深度链没有足够独立目录，线程调度可能带来回退。

**当前状态：已按用户要求默认启用快速枚举，接受枚举元信息可能滞后的取舍。** 本机 NTFS 硬链接样本中，枚举结果把旧名称读成 4 KiB、写入名称读成 8 KiB，总计 12 KiB；逐路径查询得到两个 8 KiB，总计 16 KiB。另曾观察到目录修改时间相差 1 ms。文件在开始扫描前已经写完，并非扫描中继续写入造成。benchmark 仍严格报告这些差异，不把已接受的产品取舍伪装成结果完全相等。

[Rust 文档](https://doc.rust-lang.org/std/fs/struct.DirEntry.html#method.metadata) 说明 Windows 的 `DirEntry::metadata()` 无需额外系统调用，但这不等于数据一定最新。[Microsoft 硬链接文档](https://learn.microsoft.com/en-us/windows/win32/fileio/hard-links-and-junctions) 说明不同链接名称中的大小、属性信息可能只在实际修改的那个名称上可见更新。扫描并非原子文件系统快照；正式实现使用枚举口径，原有逐路径查询仅保留为 benchmark 对照。

复现硬链接差异（本机预期退出码非零，JSON 仍会写出）：

```powershell
cargo bench -p orion-core --features bench-internals --bench scan -- --baseline legacy --candidate enumerated --suite hardlink --runs 4 --warmups 1 --label hardlink-correctness --output target/bench-results/hardlink-correctness.json
```

## 2026-09-20 本机结果（启用前的历史实验）

Windows x86-64、Rust 1.92.0、优化构建，每种实现预热 2 次后测量 8 次。A/A 中三组的配对耗时下降分别为 -2.7%、+0.7%、-1.6%，说明小幅变化不足以证明优化有效。单轮仍有明显波动，应查看原始样本与 MAD。

| 数据集 | 原查询耗时中位数 | 枚举缓存耗时中位数 | 配对加速比 | 一致性 |
| --- | ---: | ---: | ---: | --- |
| wide：12,000 文件 | 430.24 ms | 18.49 ms | 23.30× | 通过 |
| tree：4,096 文件 / 341 目录 | — | — | 不报告 | 条目元信息指纹不同，计数及总大小相同 |
| deep：512 文件 / 33 目录 | 30.47 ms | 8.01 ms | 3.79× | 通过 |
| 本项目 node_modules：6,885 文件 / 370 目录 | 364.42 ms | 84.69 ms | 4.39× | 已扫描条目及错误摘要一致；双方均报告 1 项跳过/错误，结果不完整 |

`metadata-ab.json` 整体为 `valid: false`，因为 tree 未通过；上表仅单独列出通过的组。真实目录结果仅代表这个稳定目录，不能推算整盘耗时。历史上同一硬链接 benchmark 样本对 `current` 通过、对 `enumerated` 拒绝；启用后 `current` 也会报告这一差异。逐路径查询的精确大小测试仍保留为对照，正式扫描另验证静态硬链接按目录项计数。

本次可追溯报告保存在仓库中：

- [A/A 基线](benchmarks/2026-09-20/baseline-aa.json)
- [metadata A/B（包含失败组）](benchmarks/2026-09-20/metadata-ab.json)
- [真实目录 A/B](benchmarks/2026-09-20/real-node-modules.json)
- [硬链接反例](benchmarks/2026-09-20/hardlink-correctness.json) / [同一样本的正式实现检查](benchmarks/2026-09-20/hardlink-baseline.json)

报告中的临时样本目录已清理；重新执行命令会按相同配方创建新目录。早期未加入候选优先预检的探索结果不作为本表依据。

验证：正常配置及 `bench-internals` 配置的 Release 后端测试均为 7 项通过；benchmark 的 Clippy 检查通过。A/B 的 tree 与硬链接实验退出非零是完整性检查拒绝候选的预期结果，不作为通过的性能结论。
