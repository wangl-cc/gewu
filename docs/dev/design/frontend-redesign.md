# Gewu 前端重设计：FFI · GewuKit · App

> **Status:** implemented in #17 (merged 2026-11-21). Written 2026-11-20 during the work; the last two sections record where the implementation departed from the plan and the second-round layering decisions. This is a historical design document: it is kept as written and is not updated with the code. For the current model, see the [architecture](../architecture.md) and the [development guide](../guide.md).

## 摘要

结论：新增 Swift 模块 GewuKit 作为唯一能接触 UniFFI 的一层，把 App 重建成「窗口 → 表格会话 → 各面板」三层清晰模型，FFI 只做小幅重塑，Core 不加前端概念。基线为 PR #13 头 e2bd69c（已在最新 main a5edb3f 之上）。

核心决定：

- **编译器强制的边界已实验验证**：把 UniFFI 生成的 gewu.swift 去掉 public/open 后编进 GewuKit，Swift 6 模式零错误零警告；App 侧 `import GewuKit` 后引用 `GewuDataset` 报 "cannot find type"。
- **三个领域类型保留各自的失效身份**：`TableSource`（≈Dataset）、`RowSet`（≈FilteredView）、`DisplayRows`（≈OrderedView），外加串行化 FFI 的 `actor TableSession`。
- **FFI 只改三处**：列文本与源行号改为零拷贝字节句柄；复制走同一句柄；导出进度保持 Core 的原子计数，由 GewuKit 包成 AsyncStream（与协调说明里的「回调」不同，理由见 FFI 一节）。
- **App 侧**：DocumentModel 拆分并去掉对 DataTableController 的反向依赖；全部统一到 @Observable；计数器事件、票据、RequestGate、OperationQueue 改为结构化并发；切到 Swift 6 语言模式并去掉 3 处 @unchecked Sendable。

撤销已定为不做。需要你决定的还有 4 项（见「需要你决定的问题」）：状态恢复、本地化、NSDocument，以及导出进度用轮询还是回调。

## 本 PR 范围

边界是「重建模型，并把被重写的代码的行为做对」，不加新功能。下表中的编号对应 TODO.md。

| 类别 | TODO 项 | 本 PR 做什么 |
| --- | --- | --- |
| 重设计直接解决 | T19、T27、T28、T29、T34、T09 的重复状态 | 实现并勾掉 |
| 重写会碰到的代码 | T20 列宽、T21 Find 高亮配色、T25 读取失败与超大块 | 在新代码里把行为做对并验证 |
| 只差实机验收 | T01、T52、T04、T17、T22、T23、T24、T10 | 不改交互，重写后实机走一遍，确认的勾掉，确认不了的写进 PR |
| 不在本 PR | T05–T08、T13、T14（新功能）；T26、T30（需先测量）；Core 与 README 各项；T18 | 保持原样，只更正过时描述 |

## 现状核实

以下都是在 e2bd69c 上读代码或跑实验得到的事实；「推断」会单独标出。App 源码共 7337 行，97 个测试。

| 问题 | 证据 | 影响 |
| --- | --- | --- |
| 没有领域层，FFI 类型直达视图 | 22 个源文件直接引用 `GewuFilteredView`/`ColumnId`/`Histogram` 等；`ColumnFilterView` 用 `ColumnCapabilities`、`FilterChoices` | 改 FFI 必然波及 UI；UniFFI 的 `GewuError` 需要 `userMessage` 扩展才能展示 |
| DocumentModel 是上帝对象（520 行） | 同时管列可见性、草稿查询、Inspector 页、图表轴、Sheet 开关、错误、导出、复制 | 任何功能都要改它 |
| 依赖倒置 | `weak var table: DataTableController?`；`refresh()` 在提交前循环调用 `table.loadViewport` 预读视口 | 模型测试离不开控制器；控制器挂载时机影响查询语义 |
| 重复状态 | Inspector 显隐同时在 `WorkbookModel.showsInspector` 和 `DocumentModel.inspectorVisible`，靠 `onChange` 同步；`InspectorSettings` 注释写「window-owned」但实际每个 DocumentModel 一份，切 sheet 即丢失；两条错误 alert 通道 | 状态不一致、行为与注释不符 |
| 计数器当事件 | `searchFocusRequest`、`inspectorRequest`、`FindModel.revision`/`highlightRevision` | 事件语义隐含，容易漏订阅 |
| 六种并发风格 | OperationQueue（Viewer）、DispatchQueue + RequestGate（Workbook、Find）、actor（TableSegmentReader）、Task.detached（Copy、Export、Scatter）、手写票据（Inspector、Scatter、Epoch）、DispatchQueue.main.async（DataTable） | 取消与发布语义各写一遍 |
| Swift 5.0，无严格并发 | `project.yml` 的 `SWIFT_VERSION: '5.0'`；`@unchecked Sendable` 3 处：RequestGate、ViewerScan、ColumnWidths.Request | 数据竞争只靠人工保证 |
| 观察框架混用 | ObservableObject：WorkbookModel、ExportModel、InspectorSettings、AppUpdater、FileCoordinator；其余 @Observable；DocumentModel 里还有手写 access/withMutation | 两套更新机制，T19 的根源 |
| 偏好键分散 | 10 个字符串键分布在 8 个文件，`ShowRowNumbers` 和 `RowNumberBase` 各重复 3 次并各写默认值 | T27 |
| 模型里硬编码用户文案 | `DocumentModel.status/subtitle`、`FindModel.status`、`CopyModel.message`、错误前缀字符串 | 无法本地化，模型与展示耦合 |
| DataTable.swift 1007 行 | Representable 没有 Coordinator，控制器同时是数据源、代理、列宽测量、菜单、弹窗 | 难测、难审 |

FFI 层事实：

- `FormattedColumnSegment` 是三个 `Vec<u8>` 字段的 Record。UniFFI 0.32.2 的 Swift 侧 `readBytes` 先分配 `[UInt8]` 再拷贝，然后转 `Data`（生成代码第 146 行起）。
- `source_rows` 返回 `Vec<u64>`，Swift 逐元素解码。Core 里行号是 `RowIds::Selected(Arc<[u32]>)` 或无分配的 `All(n)`。
- UniFFI 0.32.2 的零拷贝 `&[u8]` 只支持参数位置，不支持返回值；生成器没有「生成 internal 代码」的配置项（已查 `gen_swift::Config`）。
- 导出进度是 Core `Export` 里的 `AtomicU64`，FFI 暴露 `progress()`，Swift 每 150 ms 轮询。

## 分层与模块边界

四层依赖严格单向，每层只认下一层；表格的 64 行块、LRU、视口策略属于 App，GewuKit 只提供按任意行区间读取的接口。

> （原文此处为嵌入图示：分层 · 4 层，GewuKit 为唯一 FFI 边界，未迁移）

App 与 GewuKit 之间的边界由编译器强制：生成代码在 GewuKit 内是 internal，App 写出任何 FFI 类型名都无法编译（已用 swiftc 实测）。Core 本 PR 不改；FFI 只做上一节列出的形状调整。

## FFI 层改造

FFI 保持「薄、同步、无状态」：只在实测或已核实有代价的地方改形状，所有调度仍归 Swift。

### 1. 字节句柄 `ByteBuffer`

新增 UniFFI 对象（UniFFI 对象本身已在 Arc 里，所以内部直接持有 Core 交出的 Vec；转成 Arc‹[u8]› 会多一次 memcpy，所以不转），而不是持有 `Arc<[u8]>`，只暴露 `address() -> u64` 和 `len() -> u64`。Swift 在 GewuKit 内用 `UnsafeRawBufferPointer(start:count:)` 包装，缓冲区生命周期绑定在持有该对象的 Swift 值上（不用 `Data(bytesNoCopy:)`，避免 deallocator 语义与 UniFFI 释放顺序交织）。这是全 FFI 唯一的 unsafe 边界，放在一个文件里，写清 SAFETY 注释。

- `format_column(column, start, count)` 返回 `ColumnText { row_count, text: ByteBuffer, offsets: ByteBuffer, validity: ByteBuffer }`。Core 的 `FormattedColumnSegment::into_parts` 已给出三个 Vec，`直接移入句柄`；offsets 从 `Vec<u32>` 转字节不再逐元素 `flat_map`，在 little-endian 目标上直接重解释。
- `source_rows(start, count)` 返回 `ByteBuffer`（以 u32 解释）。`RowIds` 的行号上限是 u32，去掉 u64 加宽。对于 `All(n)` 这是一次小分配；整段共享 `Arc<[u32]>` 的切片需要 Core 给出子范围的 Arc，收益不明确，不做。
- 复制继续按列、每次 4096 行读取，但拼 TSV 时直接从句柄缓冲区拷字节，不再逐格建 `String`。协调说明里「零拷贝只对复制明显划算」的判断我同意：视口路径不是拷贝瓶颈，但统一用句柄可以删掉 Swift 侧的 `Data` 解码扩展，模型更简单。

### 2. 错误

`GewuError` 的形状保留（它已经是结构化的 kind）。变化在 GewuKit：按调用方能处理的语义重新分组为 `OpenError`、`FilterInputError`、`FindInputError`、`AnalysisError`、`ExportError`，都实现 `LocalizedError`，`Internal` 单独为 `GewuKitError.internalFailure`。Swift 的 `CancellationError` 与 Core 的 `Cancelled` 在 GewuKit 里统一成 `CancellationError`，App 不再需要 `isCancellation`。

### 3. 导出进度（与协调说明不同的建议）

Core 的进度是 `AtomicU64`，写入方每个 chunk 更新一次。改成回调意味着 Core 要接受一个观察者（新增前端驱动的接口），而 UI 只需要按帧率刷新，回调还要自己节流。所以我建议不改 Core 和 FFI：GewuKit 的 `ExportJob.progress` 是一个 `AsyncStream<ExportProgress>`，内部在写入期间按固定间隔读原子计数，写入结束即 finish。App 看到的是事件流，轮询细节不外泄。若你仍希望回调，改动点在 Core `WriteProgress::advance`，量不大。

### 4. 其他小整理

- 删掉 `open_sheet`（与 `open_with_options` 重复），只留 `open(path, options)`。
- `find_limits()` 留在 FFI，由 GewuKit 内部使用。
- 不引入 UniFFI async 导出：它需要 Rust 侧执行器或阻塞线程桥接，而 Swift 侧调度已经足够，且 Core 扫描不支持协作式取消。

## GewuKit API

GewuKit 按 App 的概念建模，不是 FFI 的一对一镜像：它对外只有 Swift 值类型、可比较身份的引用类型和 async 方法，热路径上不做逐格转换。

### 三个带身份的类型

| 类型 | 对应 | 持有 | 失效什么 |
| --- | --- | --- | --- |
| `TableSource` | GewuDataset | `info: TableInfo`（列、文件元数据）、`structure(of:)` | 换文件、换 sheet、改表头选项 |
| `RowSet` | GewuFilteredView | `source`、`filters`、`count`；所有分析（nulls、statistics、distribution、values、lengths、scatter）；`filterOptions(for:)` 内部做 facet | 筛选变化；分析结果缓存挂在它上，随它释放 |
| `DisplayRows` | GewuOrderedView | `rowSet`、`sorts`、`count`；`text(column:rows:)`、`sourceRows(_:)`、`find(_:in:)`、`export(columns:format:)`、`copyText(...)` | 排序或筛选变化；表格文本缓存、Find 与它绑定 |

三者都是 `final class ... : Sendable`（不可变，内部只有 let 和自带锁的缓存），并暴露 `id: ObjectIdentifier`，App 的身份判断统一写成 `rowSet.id == other.id`，不再到处 `===`。`DisplayRows` 持有它的 `rowSet`，所以「当前显示」只需一个值就能推出两层身份。

### 值类型

- 查询：`ColumnID`、`ColumnFilter`、`FilterCondition`、`SortKey`、`TableQuery { filters, sorts }`（Equatable，用于判断「是否需要重算」）。列可见性不在 `TableQuery` 里，它是纯前端状态。
- 元数据：`TableInfo`、`ColumnInfo`（含 `capabilities`、`analyses`）、`FileMetadata`。
- 分析结果：`NumericStatistics`、`Histogram`、`ValueSummary`、`LengthSummary`、`ScatterSample`、`FilterOptions`。这些是一次性的小结果，转换成本可忽略；顺便在这里解决 T34，把日期坐标直接给成 `Date`（若 FFI 已提供可用坐标；否则保持现状，不扩大范围）。
- 热路径：`ColumnText` 是缓冲区支撑的结构，`subscript(row) -> Substring?` 或 `utf8(row) -> UnsafeRawBufferPointer`，只在单元格绑定时解码一个格。`RowNumbers` 同理，是 u32 视图。

### TableSession

```swift
public actor TableSession {
    public nonisolated let source: TableSource
    public static func open(_ url: URL, options: OpenOptions) async throws -> TableSession
    /// 筛选未变则复用 current.rowSet，只重排序；两者都未变则直接返回 current
    public func apply(_ query: TableQuery, after current: DisplayRows) async throws -> DisplayRows
}
```

actor 只用于串行化「查询」这一类重活。它的方法把同步 FFI 调用放到一个专用串行执行器上（自定义 `SerialExecutor`，而不是占用 Swift 协作线程池），并在进入 FFI 前检查取消。分析、文本读取、Find 各自有独立的串行执行器，这样长分析不会挡住滚动（现在 Viewer 也是查询、分析两个队列；这一点设计保持不变）。

开放问题（实现时测）：是否需要「每窗口一条」还是「全局一条」分析队列。现状是每窗口一条，我先沿用。

### 分析缓存迁入 RowSet

`Viewer.Epoch` 的 32 槽 LRU、票据、`active` 取消表改为 `RowSet` 内部的「去重 + 缓存」：同一参数的进行中请求共享一个 `Task`，完成后入 LRU。`RowSet` 被替换后无人持有则内存自然释放，不再需要显式 `epoch.cancel()`。各面板取消自己的等待不会取消共享计算（与 T24 已定的语义一致）；只有当所有等待者都取消且计算尚未开始时才不进入 Core。

## App 层模型重建

原则：每个状态只有一个所有者，所有者的生命周期等于状态的生命周期；模型不引用视图，视图向模型发意图。

### 所有权

| 模型 | 生命周期 | 拥有 |
| --- | --- | --- |
| `AppModel` | 进程 | `FileCoordinator`（打开、最近文件）、`AppUpdater`、`Preferences` |
| `WindowModel`（替代 WorkbookModel） | 窗口 | URL、打开状态机 `Loading / Loaded(TableModel) / Failed(error)`、sheet 列表、导入选项、Inspector 显隐与页、`InspectorSettings`（真正窗口级，切 sheet 保留）、窗口级错误通道 |
| `TableModel`（替代 DocumentModel 的数据部分 + Viewer） | 一张已打开的表 | `TableSession`、`displayed: DisplayRows`、`draft: TableQuery`、`visibleColumns`、选中列、分析列、图表轴 |
| `FindModel` | 表 | 查询文本、当前命中、高亮位图缓存；只认 `DisplayRows` 和列 |
| `CopyModel` / `ExportModel` | 表 | 各自的任务和状态 |
| `InspectorModel` / `ScatterModel` | 面板视图（@State） | 当前展示的结果；计算与缓存在 `RowSet` |

### 去掉反向依赖

现在 `refresh()` 先算视图，再通过 `weak table` 预读目标视口文本，最后一次性提交，目的是替换时不闪白。这个目的保留，但归属反转：`TableModel` 只发布 `pending: DisplayRows?`；表格控制器观察到 pending 后自己预读视口，就绪后调用 `model.commit(pending)`。模型在没有表格时（测试、窗口尚未挂载）由一个可注入的「提交门」直接提交。这样模型测试不再需要控制器。

### 事件与意图

- `searchFocusRequest`、`inspectorRequest` 这类计数器改为表格窗口内的 `@FocusState` 和直接修改 `WindowModel.inspector` 状态；确实需要「一次性」语义时用带 ID 的意图值，由接收方消费后清空。
- Find 的 `revision`/`highlightRevision` 是为了让 SwiftUI 调用 representable 的 update。改为让控制器通过 Coordinator 直接观察 `FindModel`（`withObservationTracking`），只重绘受影响的可见单元格。
- `DataTableController` 拆成：Coordinator（桥接 SwiftUI）、`TableDataSource`（视口、单元格绑定）、`ColumnWidthFitter`、`TableMenus`。每个文件一个职责，目标是没有超过 400 行的文件。

### 表格缓存的归属

64 行块、LRU、视口保留是表格的展示策略，留在 App 的 Table 目录，不进 GewuKit 也不进 Core。缓存键增加 `DisplayRows.id`，替换 `DisplayRows` 时旧键自然失效，去掉手写 `version` 计数。失败块的重试语义（T25）不在本 PR 范围，迁移时保持现有行为。

### 偏好与文案

- `Preferences`：一个枚举定义所有键、类型、默认值和有效范围（字号 10–20、行高 24–36 现在写在 `DataTable.body`）。视图用包装好的 `@AppStorage(Preferences.showRowNumbers)`，模型用注入的 `UserDefaults`。
- 模型不再产出展示字符串：`status` 变成 `RowCounts { shown, total, columns }`，`FindModel.status` 变成枚举，形成文字在视图层。是否同时做 String Catalog 本地化，见决策项。

### 观察框架

全部改为 `@Observable`。`AppUpdater` 内部用 KVO（`observe(\.canCheckForUpdates)`）写自己的属性，对外仅是 @Observable，去掉 Combine。`ExportSelection` 作为 NSSavePanel 代理由 `ExportModel` 在面板打开期间显式持有（T28）。`.focusedSceneObject` 改为 `.focusedSceneValue`。

### 其他现代化

下面几项随重写一起处理：

| 现状 | 改为 |
| --- | --- |
| NotificationCenter 用 selector 加 `deinit` removeObserver（DataTable、RowGutterView）；窗口激活监听用 `object: nil`，等于监听所有窗口 | `notifications(named:object:)` 的 async 序列，挂在视图生命周期的 Task 上，只监听自己的窗口；顺带解决 T28 的 gutter 重新挂载 |
| `DispatchQueue.main.async`（DataTable 2 处） | MainActor Task 或直接同步调用 |
| `NSLock` | Synchronization 的 `Mutex` |
| `Task.detached`（Copy、Export、Scatter） | 由 GewuKit 执行器承担，App 内不再出现 |
| `NumberFormatter`、`ByteCountFormatter` | FormatStyle（`.formatted()`） |
| 散点图用 `ISO8601DateFormatter` 反解析标签（T34） | GewuKit 直接给 `Date`。前提是确认 Core 散点坐标对日期/时间轴的单位；若未定义，FFI 补一个单位字段 |

保留不改：NSTableView（大表虚拟化与性能）、FrameTests/UITests 的 XCTest（性能指标与 UI 测试仅 XCTest 支持）、菜单的 `@objc` selector（AppKit 的正常写法）。

## 并发模型

一套规则替代现在的六种写法：UI 模型在 `@MainActor`，重活在 GewuKit 的串行执行器上，两者之间只通过 async 调用和结构化取消通信。

1. **语言模式**：App、GewuKit、测试全部 Swift 6（`SWIFT_VERSION: '6.0'`），开启完整并发检查。生成的 UniFFI 代码在 Swift 6 下已实测无警告。
2. **执行器**：GewuKit 内置三个串行执行器（查询、分析、表格读取），均为每个 `TableSession` 一组。同步 FFI 在专用 `DispatchQueue` 上跑，不占 Swift 协作线程池（一个长扫描占用协作线程会拖慢其他任务，这是 T29 的问题）。TableSession 的 unownedExecutor 就是查询队列（DispatchSerialQueue 本身可作 SerialExecutor）。队列对 App 不可见。
3. **取消**：Core 的内存扫描不可中断，所以取消语义固定为「未开始的不进 Core，已开始的跑完但结果不发布」。这由 GewuKit 的一个小函数实现（`withTaskCancellationHandler` + 入队时检查），取代 `RequestGate`、`ViewerScan`、各处的 continuation 样板。只有导出支持中途取消（Core 已有）。
4. **发布身份**：票据和请求计数全部删除。面板用 `.task(id: request)`，`request` 是一个包含 `rowSet.id`、列、参数的 Equatable 值；SwiftUI 在 id 变化或视图消失时自动取消旧任务，所以没有「旧结果回写」。`TableModel` 的查询任务、`FindModel` 的导航任务是模型内部的单个 `Task` 属性，替换时 cancel，发布前比较 `DisplayRows.id`。
5. **防抖**：用 `try await Task.sleep` 放在任务开头，由取消自然实现。Find 输入和筛选各一处。
6. **Sendable**：去掉全部 3 处 `@unchecked Sendable`。唯一保留的 `@unchecked` 在 GewuKit 内部：包装 UniFFI 对象和字节句柄的类型（UniFFI 生成的对象本身已是 `@unchecked Sendable`，Rust 侧保证线程安全）。分析缓存用 `Mutex`（Synchronization 框架，macOS 15 可用）。
7. **平台限制**：部署目标 macOS 15，`Observations` 异步序列要 macOS 26，所以控制器观察模型用 `withObservationTracking` 而不是它。

## 迁移步骤与删除清单

一个 PR，按提交分步，每步都能编译并通过测试，方便你逐步审。App 的模型层是重写，视图层以替换类型为主，FFI 改动较小。

1. **工程结构**：`project.yml` 新增 `GewuKit` 静态 framework 目标和 `GewuKitTests`。Rust 构建脚本移到 GewuKit 目标，生成后把 gewu.swift 的 public/open 改为 internal（一条 perl 替换，已实测）。App 只链接 GewuKit。不用 SwiftPM：它需要把 Rust 库打成 xcframework 作为 binary target，增加一套构建。
2. **FFI 重塑**：`ByteBuffer`、`ColumnText`、`source_rows` 改 u32、删 `open_sheet`。Rust 测试与 view-smoke.swift 同步。
3. **GewuKit**：类型、`TableSession`、执行器与取消、分析缓存、错误映射、导出与复制。从现有 App 测试里搬过来属于这一层的（QuerySpec、Viewer 缓存、取消隔离）。
4. **App 切换到 GewuKit**：先把模型迁到新类型（TableModel、WindowModel），再迁视图。这一步结束时 App 里不再有任何 FFI 名字，由编译器保证。
5. **表格拆分**：DataTable.swift 拆文件、加 Coordinator、反转预读依赖。
6. **并发与观察统一**：Swift 6、@Observable、删计数器事件。
7. **偏好与文案**：`Preferences`、模型不再产出字符串。
8. **文档**：重写 app/README.md 和 gewu-uniffi/README.md 的架构部分，TODO.md 勾掉完成的 T19/T27/T29 等并更新描述。

删除清单（实现完成时这些名字不应再存在）：

| 删除 | 由什么取代 |
| --- | --- |
| `Viewer`（含 `Epoch`、`ResultCache`、`Slot`、`ViewerScan`） | `TableSession` + `RowSet` 内缓存 |
| `RequestGate`、BackgroundWork.swift | GewuKit 的串行执行器与取消函数 |
| `DocumentModel.TableInput`、`RowQuery`、`submitted` | `TableQuery` + `TableModel` 的 pending/displayed |
| `DocumentModel.table`（weak 控制器） | 控制器观察模型 |
| `inspectorVisible`、`inspectorRequest`、`searchFocusRequest` | `WindowModel.inspector`、`@FocusState` |
| `FindModel.revision`、`highlightRevision`、两个 gate 和请求计数 | 观察 + 单个 Task |
| `InspectorModel`/`ScatterModel` 的 ticket、`Context` | `.task(id:)` |
| `TableSegmentLoader` 的 `version`/`lastVersion` | 缓存键带 `DisplayRows.id` |
| `FormattedColumnSegment` Swift 扩展、ErrorPresentation.swift | `ColumnText`、GewuKit 错误类型 |
| 10 处字符串偏好键 | `Preferences` |
| WorkbookModel 与 ExportModel 的 Combine/ObservableObject | @Observable |
| FFI `open_sheet` | `open(path, options)` |

## 需要你决定的问题

前三项是范围问题，我的建议都是「本 PR 只把模型做成能支持它的形状，功能本身另开」，因为它们各自有独立的验收。

| 问题 | 选项 | 我的建议 |
| --- | --- | --- |
| 撤销（筛选、排序、列显隐） | 已定：不做 | 撤销通常是针对编辑数据的，只读查看器不需要，设计里不为它预留任何东西。`TableQuery` 仍然是值类型，理由是它本身有用（判断是否需要重算、可编码），和撤销无关 |
| 状态恢复（T13：切 sheet、重启恢复查询与布局） | 本 PR / 后续 | 后续；本 PR 让 `TableQuery`、列、选中都 Codable，并把 `InspectorSettings` 改为窗口级 |
| 本地化 | 本 PR 加 String Catalog / 只把文案移出模型 | 只移出模型；是否做中文界面是产品决定 |
| NSDocument | 采用 / 不采用 | 不采用：Gewu 是只读查看器，NSDocument 的保存、版本、自动保存都用不上，而现有 WindowGroup(for: URL) 已有恢复和最近文件 |
| 导出进度 | Core 加回调 / GewuKit 把轮询包成 AsyncStream | AsyncStream，理由见 FFI 一节 |

另有两点我按推断先定，若你不同意告诉我：每窗口一条分析队列（沿用现状）；不引入 UniFFI async 导出。

## 验证计划

按「测试放在拥有该行为的稳定边界」的仓库规则分层，不在 Core、FFI、GewuKit、App 重复同一矩阵。

| 层 | 验证 | 命令 |
| --- | --- | --- |
| FFI | 句柄的长度、对齐、空段；u32 行号；错误映射 | `cargo test --locked -p gewu-uniffi`、clippy、nightly fmt、view-smoke |
| GewuKit | 查询复用（筛选不变则不重算 RowSet）、分析去重与取消隔离、未开始的取消不进 Core、ColumnText 解码、导出进度流结束 | 新的 GewuKitTests |
| App 模型 | 草稿/提交、列显隐与条件清理、失败回滚、Find 导航、窗口生命周期；不再依赖控制器 | GewuTests（现有 97 个测试迁移后保留其覆盖的行为） |
| 性能 | 滚动帧时间、筛选呈现、散点悬停不退化 | GewuPerformance scheme（FilterFrameTests、ScatterPerformanceTests），前后对比 |
| 格式 | swift-format strict | CI 同款命令 |
| 实机 | 打开 CSV/Parquet/XLSX、筛选、排序、切 sheet、Find、复制、导出取消与完成、Inspector 和图表 | 本机启动 App 手动走一遍，结果写进 PR |

报告时会区分「构建通过」、「自动测试通过」和「实机验证」。

## 实现中的调整

实现时发现了更简单或更正确的做法，以下几处与上文不同，以代码为准。

| 上文写的 | 实际做法 | 原因 |
| --- | --- | --- |
| `TableSession` 是 actor | `TableSession` 是 Sendable 的 final class；串行化由每个会话的 `SerialWorker` actor（执行器是 DispatchSerialQueue）负责 | 会话本身没有可变状态，只有一种 actor 更简单 |
| 模型发布 pending，控制器预读后调 `commit` | 模型的 `displayed` 在查询成功时立即更新；表格控制器自己双缓冲，继续显示旧行直到新视口读完，再一次性切换 | 模型完全不等待视图，也不需要「提交门」。代价是状态栏比表格早几毫秒更新；视口读取失败不再撤销用户的筛选 |
| 列可见性跟随查询提交 | 列可见性是纯展示状态，立即生效；删掉 `draftVisibleColumns` | 隐藏列只会移除条件，不会失败，没必要等查询 |
| 通用 `ByteBuffer` 句柄 | `ColumnText` 与 `RowIndices` 两个对象，各自暴露 `BufferView { address, length }` | 一个文本块只需一个句柄，生命周期也更清楚 |
| `GewuDataset::open` 构造器 | 自由函数 `open_dataset`；`uniffi.toml` 给所有生成类型加 `Ffi` 前缀 | 生成代码与 GewuKit 公共类型同在一个模块，必须改名；UniFFI 0.32 的改名不作用于构造器签名（已用生成结果验证） |
| Core 不改 | Core 的 `ScatterAxisKind` 带上时间单位 `TimeUnit` | T34：Core 的散点坐标是原生单位，前端无从得知单位，这是契约缺口，不是前端概念 |
| `SortKey` | 保留 `ColumnSort` | 与 `ColumnFilter` 对称 |
| Coordinator + `withObservationTracking` | `DataTable` 每次构造一个 `TablePresentation` 值交给控制器，控制器只应用差异 | SwiftUI 自己追踪读到的状态，单向数据流更清楚 |
| `@FocusState` 控制 Find 焦点 | `WindowModel.isSearchFocused` 双向绑定到原生搜索框 | `@FocusState` 对 NSViewRepresentable 不可靠 |

## 第二轮：FFI 与分层修正（2026-11-20 讨论结论）

PR #17 的 FFI 数据传输没有达到目标，讨论后的分层原则：Core 只做通用数据语义；FFI 除了跨边界投影，还可以承担在 Rust 里表达更好的应用语义；FFI 不能污染 Core（Core 不引入 uniffi），但两层形状可以绑定；交互状态留在 Swift。

| 层 | 内容 |
| --- | --- |
| Core | facet 下的筛选选项（一个直接表达意图的方法）；Filter 空值语义显式化；`ColumnInfo.is_nested`；导出支持行子集、内存目标与字节上限，分隔文本导出可选「嵌套值写成显示文本」（文件导出也可用）；行号的只读访问器（`All(n)` 或 `Arc<[u32]>`） |
| FFI（应用语义） | FilteredView 上的分析结果缓存（`OnceLock`，取代 Swift `AnalysisCache`）；视图记住自己的 filters/sorts，提供 `apply` 复用；FindSession 内部分页、按步前后移动；散点坐标换算成秒 |
| FFI（投影） | 能用 `#[uniffi::remote]` 的结果类型全部改 remote；`ColumnId` custom type；`FileMetadata` 元组转 `KeyValue`；错误保留结构化 kind（供 i18n）；`ExportOptions` 改参数；生成代码独立成 `GewuFFI` 模块，删掉 uniffi.toml 改名，恢复 `GewuDataset::open` 构造器 |
| 数据面 | 手写 C ABI 的类型化只读视图（`const uint32_t *` 等加长度），经 UniFFI 句柄借用对象内存：行号、复制/导出到内存的结果。现算的格式化文本打包成单缓冲区按值传。删掉 BufferView、ColumnText 对象、RowIndices |
| GewuKit | 带关联值的结构化错误，文案在 Swift 合成；`TableState` 值类型状态机、`FilterDraft`（含按 UTF-8 判等的 `ExactValue`）、Find 交互状态；调度与取消 |
| App | 展示、视口缓存、表格切换、系统外壳（保存面板、剪贴板） |

复制走导出路径：选中行列导出成 TSV 到内存，嵌套值按显示文本写出，行尾与值格式跟随导出。
