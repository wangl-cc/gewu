# Parquet 查看器：Rust 核心与 Swift 界面的 API 设计

> **Status:** superseded. Written 2026-10-28, before the first commit. The Rust core, UniFFI and the native Swift table were built as planned, but the central model was replaced: the mutable `Document` with a `ViewState` and version numbers gave way to immutable views owned by the caller (#10), dictionary-key filters to exact text values (#10), cooperative cancellation was removed, and the CLI shipped then was later removed while the MCP server was never built. This is a historical design document: it is kept as written and is not updated with the code. For the current model, see the [architecture](../architecture.md).

## 设计原则

- **数据和状态都在 Rust**。打开的文件、当前排序、筛选条件、算出来的行号都由 Rust 持有。Swift 只发“意图”（按这列排序、只看这几个值），不自己维护数据。
- **接口按“表格需要什么”来设计**，不暴露 Arrow 类型。表格只需要：有几列、有几行、第 N 到 M 行的文字、某列有哪些值。
- **慢的调用是 async 且可取消，快的调用是同步的**。排序、筛选、统计是 async；取可见行是同步的，必须在毫秒内返回。
- **界面永远不空白**。新的排序结果算完之前，旧结果照常可用；算完后一次性切换。
- **用 UniFFI 生成绑定**，Swift 侧拿到的是普通的 class、struct、enum 和 `async throws` 方法。

## 核心概念

只有三个概念：

- **Document**：一个打开的文件。持有 schema、文件元数据和解码后的数据。Swift 端每个窗口对应一个 Document。
- **ViewState**：用户想怎么看。包括排序列、筛选条件、搜索词。这是一个纯数据结构，Swift 每次修改后整个发给 Rust。
- **View**：Rust 按 ViewState 算出的结果，本质上就是一个行号数组（原始第几行排在显示的第几行），再加一个版本号。显示行数就是这个数组的长度。

**版本号**是保证前后一致的关键：每次 View 更新版本号加 1。Swift 取行时带上自己认为的版本，版本对不上就返回错误，Swift 丢掉缓存重新取。这样就不会出现“行数是新的、内容是旧的”这种错位。

排序和筛选都不移动数据，只产生新的行号数组。一亿行的行号数组用 u32 存是 400 MB，几百万行只有几 MB。

## 接口定义

下面是 Rust 侧的 UniFFI 签名，Swift 绑定会自动生成。

```rust
// ---------- 数据结构 ----------
#[derive(uniffi::Record)]
pub struct ColumnInfo { pub name: String, pub type_name: String, pub nullable: bool }

#[derive(uniffi::Record)]
pub struct FileInfo {
    pub path: String,
    pub row_count: u64,
    pub columns: Vec<ColumnInfo>,
    pub row_groups: u32,
    pub file_size: u64,
    pub compression: String,
}

#[derive(uniffi::Enum)]
pub enum SortDir { Asc, Desc }

#[derive(uniffi::Record)]
pub struct SortKey { pub column: u32, pub dir: SortDir }

#[derive(uniffi::Enum)]
pub enum Filter {
    /// Excel 式勾选：keys 来自 distinct_values 返回的 key
    InValues { column: u32, keys: Vec<u64> },
    IsNull { column: u32, negate: bool },
    Contains { column: u32, text: String, case_sensitive: bool },
}

#[derive(uniffi::Record)]
pub struct ViewState {
    pub sort: Vec<SortKey>,      // 支持多列排序
    pub filters: Vec<Filter>,    // 多个条件之间是 AND
    pub search: Option<String>,  // 全表搜索
}

#[derive(uniffi::Record)]
pub struct ViewInfo { pub version: u64, pub row_count: u64 }

#[derive(uniffi::Record)]
pub struct RowBlock {
    pub version: u64,
    pub start: u64,
    pub n_rows: u32,
    pub n_cols: u32,
    pub cells: Vec<Option<String>>,  // 按行排列，None 表示空值
    pub source_rows: Vec<u64>,       // 每行在原文件里是第几行
}

#[derive(uniffi::Record)]
pub struct DistinctValue { pub key: u64, pub label: Option<String>, pub count: u64 }

#[derive(uniffi::Record)]
pub struct DistinctResult { pub values: Vec<DistinctValue>, pub truncated: bool }

#[derive(uniffi::Record)]
pub struct ColumnStats {
    pub null_count: u64,
    pub distinct_count: u64,
    pub min: Option<String>,
    pub max: Option<String>,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CoreError {
    #[error("io: {msg}")] Io { msg: String },
    #[error("unsupported: {msg}")] Unsupported { msg: String },
    #[error("cancelled")] Cancelled,
    #[error("stale version")] StaleVersion,
    #[error("internal: {msg}")] Internal { msg: String },
}

// ---------- 对象 ----------
#[derive(uniffi::Object)]
pub struct Document { /* 数据、当前 View、取消令牌…… */ }

#[uniffi::export(async_runtime = "tokio")]
impl Document {
    /// 打开文件（慢）
    #[uniffi::constructor]
    pub async fn open(path: String) -> Result<Arc<Self>, CoreError>;

    /// schema 和元数据（快）
    pub fn info(&self) -> FileInfo;

    /// 当前 View 的版本和行数（快）
    pub fn current_view(&self) -> ViewInfo;

    /// 应用排序/筛选/搜索（慢，可取消）。新调用会取消上一次还没算完的
    pub async fn apply_view(&self, state: ViewState) -> Result<ViewInfo, CoreError>;

    /// 取可见行（快，同步）。版本对不上返回 StaleVersion
    pub fn rows(&self, version: u64, start: u64, count: u32, columns: Vec<u32>)
        -> Result<RowBlock, CoreError>;

    /// 某列的不同值和数量，给勾选筛选用（慢）
    /// respect_other_filters = true 时和 Excel 一样，只统计被其他列筛选后剩下的行
    pub async fn distinct_values(&self, column: u32, respect_other_filters: bool, limit: u32)
        -> Result<DistinctResult, CoreError>;

    /// 列统计（慢）
    pub async fn column_stats(&self, column: u32) -> Result<ColumnStats, CoreError>;

    /// 把当前 View 导出成 Parquet / CSV（慢）
    pub async fn export_view(&self, path: String, format: ExportFormat) -> Result<u64, CoreError>;

    /// 把当前排序和筛选翻译成 polars 代码（快）
    pub fn view_as_code(&self, lang: CodeLang) -> String;
}
```

几个细节：

- **筛选用 key，不用字符串**。勾选列表里显示的是格式化后的文字，但浮点数、时间戳格式化后可能不唯一。key 是 Rust 内部对真实值的编号，避免筛错。
- **单元格格式化只在 Rust 做一次**，Swift、CLI、MCP 看到的文字完全一致。
- `source_rows` 让界面可以显示“原始行号”，也方便以后做“定位到这一行”。

## 线程与取消

- Rust 内部跑一个 tokio 运行时。排序、筛选这类吃 CPU 的活放进 `spawn_blocking` 或 rayon 线程池，不占 async 线程。
- **取消要显式做**。Swift 取消 Task 时，UniFFI 会丢弃 Rust 的 future，但已经在线程池里跑的排序不会自己停。所以每次 `apply_view` 都带一个取消令牌（`CancellationToken` 或一个原子代数），计算中每处理完一块（比如 64K 行）就检查一次，发现过期就返回 `Cancelled`。
- **新调用自动取消旧调用**。`apply_view` 开始时先把代数加 1，这样用户连点两次列头，第一次会自动中止，Swift 不用管。
- **算完再切换**。新 View 算完后用 `ArcSwap` 一次性替换，版本号加 1。计算期间 `rows()` 继续返回旧 View 的数据，表格照常滚动。
- `rows()` 是同步的，在主线程调用，所以它必须只读内存里的数据，不碰磁盘。第一版把整个文件解码进内存就能满足这一点。

## Swift 侧如何使用

Swift 这边主要是一个 NSTableView 的数据源，外加一个按块缓存行的小缓存。

```swift
@MainActor
final class TableController: NSObject, NSTableViewDataSource, NSTableViewDelegate {
    let doc: Document
    var view: ViewInfo
    var state = ViewState(sort: [], filters: [], search: nil)
    let cache = RowCache(blockSize: 256, maxBlocks: 64)   // 按 256 行一块缓存
    var viewTask: Task<Void, Never>?

    func numberOfRows(in tableView: NSTableView) -> Int { Int(view.rowCount) }

    func tableView(_ tv: NSTableView, viewFor column: NSTableColumn?, row: Int) -> NSView? {
        let block = cache.block(for: row, version: view.version) {
            try doc.rows(version: view.version, start: $0, count: 256, columns: visibleColumns)
        }
        // 从 block 里取出这一格的文字，放进复用的 NSTableCellView
    }

    func tableView(_ tv: NSTableView, sortDescriptorsDidChange old: [NSSortDescriptor]) {
        state.sort = tv.sortDescriptors.map(SortKey.init)
        refreshView()
    }

    func refreshView() {
        viewTask?.cancel()
        viewTask = Task {
            do {
                let v = try await doc.applyView(state: state)
                view = v
                cache.clear()
                tableView.reloadData()
            } catch CoreError.Cancelled {
                // 被新操作取代，忽略
            } catch { showError(error) }
        }
    }
}
```

要点：

- `rows()` 一次取 256 行而不是一格一取，把跨语言调用次数降到每滚几屏才一次。
- 缓存按版本号区分，版本变了整个清空。
- 排序期间可以在状态栏显示一个小转圈，表格本身不动。
- 勾选筛选弹窗：打开时调 `distinctValues`，用户点“确定”后把勾选的 key 写进 `state.filters` 再 `refreshView()`。

## 给 agent 用的 CLI 和 MCP

Rust 核心不依赖界面，所以 CLI 和 MCP 只是同一套 `Document` 接口外面的薄壳。工程上分成四个 crate：

- `gewu-core`：上面定义的所有逻辑，不含 UniFFI 以外的界面代码
- `gewu-ffi`：UniFFI 导出，给 Swift 用
- `gewu-cli`：命令行，输出 JSON
- `gewu-mcp`：MCP 服务器（也可以做成 `gewu mcp` 子命令）

**第一步：独立模式**。agent 自己打开文件，不需要 App 开着。

```text
gewu schema data.parquet
gewu rows data.parquet --sort price:desc --filter 'label in (cat,dog)' --limit 50
gewu distinct data.parquet label --limit 100
gewu stats data.parquet price
gewu export data.parquet --filter 'label = cat' -o cat.parquet
gewu code data.parquet --sort price:desc --filter 'label = cat'   # 输出 polars 代码
```

MCP 工具一一对应：`schema`、`read_rows`、`distinct_values`、`column_stats`、`export`。给 agent 用时有两点要注意：

- **默认限制输出行数**（比如 50 行），并在结果里告诉 agent 总行数，避免一次把百万行塞进上下文。
- **筛选在 CLI/MCP 里用值本身**（`label in (cat,dog)`），不用 key。key 只在界面的勾选弹窗里用，核心里提供一个“值 → key”的转换。

**第二步：联动模式**（以后做）。App 开着时在本地 Unix socket 上暴露同样的接口，再加两个工具：`list_windows`（当前打开了哪些文件、各自的排序和筛选）和 `set_view`（帮你设置筛选，结果直接显示在窗口里）。这样 agent 能“看到你正在看的东西”。

## 关键取舍与以后扩展

- **跨语言传字符串，不传 Arrow**。一屏大约 50 行 × 20 列 = 1000 个字符串，开销是微秒级。用 Arrow C Data Interface 会让 Swift 侧复杂很多，而且格式化逻辑会分散到两边。
- **第一版整个文件读进内存**。对几 GB 以内的文件最简单也最快。以后要支持比内存还大的文件时，内部改成按 row group 懒加载、用 DataFusion 做可以溢写磁盘的排序，接口不变。那时 `rows()` 可能需要变成“先返回占位、后台加载完再通知刷新”。
- **行号用 u32**。上限约 42 亿行，内存比 u64 省一半。
- **多格式**：`Document` 内部包一个 `Source` trait（返回 schema 和 RecordBatch）。Parquet 先做，CSV、Arrow IPC、NDJSON 以后加一个实现就行，上层全部不动。
- **只读**。接口里没有修改数据的方法，导出只写新文件。
