# Gewu 改进计划与 TODO

本文档汇总了当前关于 **UI 与交互设计** 以及 **文档与开源展示** 的改进方案和待办清单。

---

## 1. UI 与交互改进 (UI & Interaction)

当前 Gewu 的底层架构（Rust + Apache Arrow + AppKit 虚拟表格）性能表现优异，但在前端交互与 macOS 人机界面指南（HIG）规范上，存在若干功能权责错位与细节待打磨点。

### 1.1 核心搜索与过滤心智重构 (P0)

当前问题：界面入口采用标准查找（`⌘F`、`Find Next ⌘G`），但底层行为却将 `search` 作为跨列过滤条件执行全表行裁剪、联动侧边栏统计重算与 Facet 计数，破坏了前后行上下文，导致“在被过滤掉的行里继续遍历”的割裂体验。

- [ ] **理顺“查找（Find）”与“过滤（Filter）”的权责**
  - **问题**：`ViewState` 中包含了 `search: Option<String>`，`apply_view_checked` 将搜索词与列过滤一起作为行裁剪条件，导致按 `⌘F` 输入关键词时整表行数缩减、侧边栏统计重算、前后行上下文丢失。
  - **解决办法**：
    1. **查找模式（`⌘F`）脱离 `ViewState`**：从 Rust `ViewState`（行裁剪、Inspector 统计、Facet 计数与导出）中移除 `search` 字段；查找不改变当前视图行数、不触发侧边栏重算，仅在当前视图行集上对命中单元格加亮底色。
    2. **Rust 端匹配索引与跳转（`⌘G` / `⇧⌘G`）**：在 Rust 侧新增可取消的 `find_matches(version, query)` 接口，维护全表匹配坐标索引 `Vec<(u32, u16)>`（`display_row, column`）与总匹配数；Swift 端按索引 `O(1)` 纵向 + 横向滚动定位到目标单元格，并在当前焦点匹配格绘制醒目外框（Focus Ring）。
    3. **行裁剪归属于列漏斗（Column Filter）**：仅由 `ViewState.filters` 触发数据集缩减与统计更新；为 `ContainsEditor` 与 `NullFilterView` 补齐缺失的 `Reset` 按钮。
    4. **新增已激活过滤条件胶囊条（Active Filter Bar）**：在 `DocumentScene.swift` 表头上方新增水平胶囊条（当 `!model.filters.isEmpty` 时展示，如 `borough = Manhattan ×`），点击胶囊打开对应列的 `ColumnFilterView`，点击 `×` 调用 `setFilter(column, nil)`，右侧提供一键 `Clear All`。

### 1.2 表头功能补齐与手势修复 (P1)

当前问题：列无法隐藏；表头 `mouseDown` 同步修改 `@Published var selectedColumn` 触发 SwiftUI 重绘，干扰了 AppKit 原生手势追踪；切换侧边栏分析列在宽表下入口过深。

- [ ] **修复表头手势干扰与列状态标记**
  - **问题**：`FilterHeaderView.mouseDown` 在调用 `super.mouseDown(with:)` 之前同步执行 `controller.onSelectColumn?(columnIndex)` 更新 `@Published var selectedColumn`，触发 SwiftUI 视图树重绘并打断 AppKit `NSTableHeaderView` 的点击排序、拖拽重排与分割线调宽手势。
  - **解决办法**：移除 `DocumentModel.selectedColumn` 及 `FilterHeaderView.mouseDown` 中的同步状态写入，完全交由 `super.mouseDown(with:)` 原生事件循环处理排序、重排与调宽；保持点击表格与 `inspectedColumn` 解耦，当右侧 Inspector 展开时，仅在 `ColumnHeaderCell` 中对 `column == inspectedColumn` 绘制极淡强调色背景与底边 `2pt` 强调色指示线。
- [ ] **支持列隐藏（Column Hiding & Visibility）与导出联动**
  - **问题**：处理几十上百列的宽表时无法隐藏无关列，且无统一列显隐管理入口。
  - **解决办法**：在 `DocumentModel` 中维护 `hiddenColumns: Set<UInt32>` 并同步设置 `NSTableColumn.isHidden`；更新 `DataTableController.displayedColumns` 过滤掉 `isHidden` 列（使复制、导出与查找跳转自动跳过隐藏列）；在工具栏与表头右键菜单提供带搜索框的 `Customize Columns…` 气泡列表。
- [ ] **表头右键上下文菜单（Context Menu）补齐**
  - **问题**：表头不支持右键菜单，宽表下切换 Inspector 分析列或执行列级操作入口过深。
  - **解决办法**：重写 `FilterHeaderView.menu(for event: NSEvent) -> NSMenu?`，通过 `column(at:)` 定位目标列，提供 `Analyze Column`（更新 `inspectedColumn` 并展开 Inspector）、`Filter Column…`、`Sort Ascending / Descending / Clear Sort`、`Fit Column Width`、`Fit All Columns` 以及 `Hide Column` / `Show All Columns`。

### 1.3 行选择价值升级、图表双向联动与分箱修复 (P1)

当前问题：表格当前的行选择除了 `⌘C` 整行复制外没有任何探查意义；超长文本和 Parquet 嵌套结构被列宽截断后无法在软件内查看完整内容；表格与侧边栏图表相互孤立；低基数/二值数值列在直方图中仍被强行切分为数十个空 Bin。

- [ ] **保持纯粹的行选择模型，提供单元格右键上下文菜单**
  - **问题**：表格缺少右键上下文菜单，用户无法直接根据单元格当前值快速过滤或单独复制单个单元格文本。
  - **解决办法**：保持 `NSTableView` 纯行选择模型（不引入复杂的单元格焦点状态机），通过 `tableView.clickedRow` 与 `tableView.clickedColumn` 构建动态 `NSMenu`：提供 `Filter to "<value>"`（直接写入 `ColumnFilter.values` 或等值条件）、`Exclude "<value>"`、`Copy "<value>"`（仅复制该单元格原始文本）与 `Copy Row`。
- [ ] **选中行空格键预览（`Space` / `Inspect Row`）**
  - **问题**：超长字符串、JSON 文本及 Parquet `Struct` / `List` 嵌套列被表格列宽与单行高度截断后，无法在应用内查看完整内容。
  - **解决办法**：在 `DataTable` 中监听 `Space` 键（及右键菜单 `Inspect Row…`），调用 `doc.rows(version: ..., start: row, count: 1, columns: allColumns)` 获取选中行完整字段，弹出带列名搜索框的纵向详情卡片（Row Record View），支持多行自动换行与 JSON / 嵌套结构缩进展示。
- [ ] **选中行 ↔ 侧边栏图表双向定位联动（Table ↔ Chart）**
  - **问题**：主表选中行与侧边栏的直方图、散点图完全孤立，无法查看选中行在整体分布中的位置，也无法从散点图离群点反查原表行。
  - **解决办法**：
    1. **表格 → 直方图**：当 `selectedDisplayRow` 非空时，读取该行在当前分析列的数值并定位所属 Bin 下标，在 `HistogramView` 中将该柱子染为 `controlAccentColor`。
    2. **表格 → 散点图**：读取选中行在 `(xColumn, yColumn)` 的坐标值，作为独立高亮 `PointMark` 叠加在 `ScatterChart` 最上层。
    3. **散点图 → 表格**：在 `ScatterChart` 中点击数据点时，通过 Rust 接口将该点的 `sourceRow` 映射为当前视图的 `displayRow`（`Selection::index_of(source_row)`），驱动 `DataTable` 滚动并选中该行。
- [ ] **修复低基数/二值列的直方图分箱与 `Distinct` 精确度显示**
  - **问题**：`histogram.rs` 对浮点/小数及不相邻整数列一律按跨度切分 40 个 Bin，导致二值列（如 `0/100`）出现 38 个空 Bin；`values.rs` 将 `ValueSummary.estimated` 硬编码为 `true`，导致所有精确统计都显示 `≈ K` 与 `estimated`。
  - **解决办法**：在 `histogram.rs` 第一遍扫描时用容量为 `bins + 1` 的小型集合统计不同值个数 `K`，若 $K \le \text{bins}$ 则将分箱数收敛为 $K$；在 `values.rs` 中根据 Sketch 是否发生过淘汰或哈希碰撞返回真实的 `estimated` 布尔值，且当数值列 $K \le 10$ 时默认展开 `Values` 面板。

### 1.4 右侧 Inspector 布局、工具栏与底栏规范化 (P2)

当前问题：列分析面板（Inspector）被放在了窗口左侧的主导航栏位置，且每次打开新文件默认关闭、不记忆状态；顶部工具栏缺少核心功能入口；底栏左侧混合了动态长度的提示文本且未使用等宽数字字体，导致右侧紧邻的 `Clear Filters` 按钮频繁左右跳动。

- [ ] **将 Inspector 移至窗口右侧，并改为“默认打开 + 记忆状态”**
  - **问题**：`DocumentScene.swift` 使用 `HSplitView` 将 `ColumnInfoView` 放在左侧（违反 macOS HIG 右侧 Inspector 惯例），且每次新建 `WorkbookModel` 时默认收起。
  - **解决办法**：将 `ColumnInfoView` 移至主表格右侧（使用原生 `.inspector(isPresented:)` 或右侧分栏），通过 `UserDefaults.standard.register(defaults: ["ShowColumnInspector": true])` 默认开启，并在切换显隐时写回 `UserDefaults`。
- [ ] **顶部工具栏（Toolbar）补齐核心操作入口**
  - **问题**：顶部工具栏仅有搜索框，列管理、导出与 Inspector 开关只能去菜单栏或用快捷键查找。
  - **解决办法**：在 `DocumentScene.swift` 的 `.toolbar` 中新增 `Columns`（弹出列显隐气泡）、`Export`（触发 `model.exportFile()`）和最右侧 `Inspector`（`sidebar.right` 图标，切换 `workbook.showsInspector`）三个标准 `ToolbarItem`。
- [ ] **收敛底部状态栏并融合多工作表（XLSX Worksheet）切换器**
  - **问题**：底栏数字未用等宽字体导致 `Clear Filters` 按钮随行号跳动；多工作表 `.xlsx` 在底栏下方额外堆叠了一整行 `worksheetBar`。
  - **解决办法**：为底栏所有统计数字添加 `.monospacedDigit()`，移除 `row X in file` 与快捷键长提示，将 `Clear Filters` 移至顶部 Active Filter Bar；删除独立的 `worksheetBar`，在 `StatusBarView` 最左侧内嵌紧凑的 `Menu` 下拉组件（`[ ⊞ Sheet1 ▾ ]`）用于切换 `.xlsx` 工作表。

### 1.5 启动行为修复、XLSX 子表切换加速与原生体验提升 (P2/P3)

当前问题：无文件启动 App 时会错误弹出 `Settings` 窗口并拦截文件选择面板；切换 `.xlsx` 工作表时每次都会销毁旧表并从磁盘重新解压解析全局字符串表（`sharedStrings.xml`），且首次打开时重复解析两次，导致即使是很小的子表切换也有明显延迟。

- [x] **修复无文件启动时误弹 `Settings` 窗口的问题（保持 Preview 风格直弹 `NSOpenPanel`）**
  - **问题**：`GewuApp.swift` 将 `Settings` 放在 `WindowGroup` 之前，且 `AppDelegate` 的 `hasExistingWindows` 未排除非文档窗口，导致空启动时误弹设置窗口并跳过文件选择框。
  - **解决办法**：将 `WindowGroup("Gewu", id: "viewer", ...)` 移至首位，为 `Settings` 增加 `.defaultLaunchBehavior(.suppressed)` 与 `.restorationBehavior(.disabled)`，并在 `AppDelegate.hasExistingWindows` 中排除 `Settings` / `About` / `Sparkle` 窗口。
- [ ] **XLSX 子表按需缓存与底层工作簿句柄复用**
  - **问题**：首次打开 `.xlsx` 时先调 `xlsxSheetNames` 再调 `openDocument` 重复解压两次；切换子表时每次销毁旧 `DocumentModel` 并重新从磁盘解析整个 ZIP 与 `sharedStrings.xml`。
  - **解决办法**：在 `WorkbookModel` 中新增 `private var sheetCache: [String: DocumentModel] = [:]` 缓存已打开过的子表（瞬间切回并保留各自的列宽、过滤与滚动位置）；在 Rust 侧新增 `XlsxWorkbook` 句柄持有已打开的 `calamine::Xlsx` 实例（复用已解析的 `sharedStrings.xml`），避免切换未缓存子表时重复解压文件。
- [ ] **支持原生窗口标签页（Window Tabs）**
  - **问题**：打开多个数据文件时会散落多个独立窗口，无法像 Finder / Preview 一样合并为单窗口标签页。
  - **解决办法**：在 `AppDelegate.applicationDidFinishLaunching`（或窗口挂载时）设置 `NSWindow.allowsAutomaticWindowTabbing = true`，并在菜单栏 `Window` 下启用系统原生标签页合并命令（`Merge All Windows` / `Show Tab Bar`）。
- [ ] **自动列宽策略优化**
  - **问题**：`DataTable.swift` 硬编码了前 32 列、前 32 行采样以及 `maxAutoFitWidth = 320` 上限，导致第 33 列之后的列完全不做自适应，且长文本列偏窄。
  - **解决办法**：结合 1.6 的同步首屏列宽采样，对所有初始落入可视区域（以及横向滚动新进入可视区域但尚未测量过）的列按需根据已缓存的 32 行样本计算列宽，并将长文本列的默认自动列宽上限适度放宽（如 `420pt`）。

### 1.6 Swift 前端脆弱写法与最佳实践整治 (Code Quality & Best Practices)

当前问题：Swift 前端在视图生命周期身份（View Identity）、异步状态同步、AppKit 桥接细节、热路径性能及偏好设置管理上存在若干脆弱写法（Fragile Patterns），容易在后续迭代或边界场景下诱发隐蔽 Bug。

- [ ] **移除 `ColumnAnalysisView` 上多余的 `.id(column)` 强制销毁修饰符**
  - **问题**：`ColumnInfoView.swift` 中对 `ColumnAnalysisView` 附加了 `.id(column)`，导致每次切换被分析列时整个子视图树与 `@StateObject InspectorModel` 被连根销毁重建，用户的折叠/展开状态（`expanded`）、分箱数（`histogramBins`）与坐标轴比例（`histogramScale` / `frequencyScale`）全部丢失，且使 `InspectorModel` 内部已设计的 `Context` 切换逻辑形同虚设。
  - **解决办法**：移除 `ColumnInfoView.swift` 第 43 行的 `.id(column)`，依赖现有的 `.task(id: request)` 与 `InspectorModel.Context` 自动感知 `column` 变化并取消旧任务；在 `onChange(of: column)` 中仅按新列的 `capabilities` 平滑修正不兼容的展开项或对数比例尺。
- [ ] **消除搜索防抖（Debounce）与回车跳转/`selectedDisplayRow` 清空的竞态**
  - **问题**：`DocumentScene.swift` 中 `onChange(of: model.searchText)` 使用 150ms 防抖异步刷新，而 `onSubmit(of: .search)` 立即同步调用 `findNext()`；且 `DocumentModel.refresh()` 完成后会无条件将 `selectedDisplayRow = nil` 清空，导致输入搜索词后立即按回车跳转会被随后的异步刷新抹掉选中状态。
  - **解决办法**：结合 1.1 将查找从 `ViewState` 过滤管线中剥离（查找不再调用 `refresh()` 重建视图、不再清空 `selectedDisplayRow`）；若查找索引仍在构建中，回车跳转（`findNext`）应等待当前查找任务完成后再定位到首个匹配项。
- [ ] **加固命令行参数解析与文件存在性校验**
  - **问题**：`AppDelegate.swift` 中 `applicationDidFinishLaunching` 遍历 `CommandLine.arguments` 时，遇到任何以 `-` 开头的参数都会盲目调用 `_ = args.next()` 吞掉下一个参数；若启动时带有无参布尔 Flag（如 `-NSDocumentRevisionsDebugMode`），紧随其后的真实文件路径会被误吞。
  - **解决办法**：仅对已知带值系统参数（如 `-NS...` / `-Apple...`）成对跳过，对非 `-` 开头的参数先通过 `FileManager.default.fileExists(atPath:)` 校验路径存在性后再调用 `FileCoordinator.shared.open(...)`。
- [ ] **修复嵌套 `ObservableObject`（`ExportModel` / `CopyModel`）状态未向 `DocumentModel` 转发的问题**
  - **问题**：`DocumentModel` 内部持有 `lazy var export` 与 `lazy var copying` 两个嵌套 `ObservableObject`，并暴露了计算属性 `var canExport: Bool { !isBusy && export.isIdle }`；但由于 SwiftUI 不会自动订阅嵌套 `ObservableObject` 的 `objectWillChange`，任何仅观察 `DocumentModel` 的视图（如工具栏按钮）在导出状态变化时不会收到重绘通知。
  - **解决办法**：在 `DocumentModel.init` 中将 `export` 与 `copying` 改为非 `lazy` 属性，并通过 Combine `export.objectWillChange.sink { [weak self] _ in self?.objectWillChange.send() }` 转发变更通知（或统一迁移至 Swift `@Observable` 宏）。
- [ ] **消除表格异步自动列宽导致的二次跳动，并优化双击自适应列宽**
  - **问题**：`DataTable.swift` 在 `buildColumns` 时先用空数据计算表头宽度，随后在异步 `loadNext()` 返回数据块时动态修改前 32 列的宽度（`initialFitsRemaining`），导致首屏渲染后表格产生肉眼可见的横向跳动，甚至可能覆盖用户刚刚手动拖拽调整过的列宽；且双击分割线 `sizeToFitWidthOfColumn` 先同步返回旧宽度再异步改宽度。
  - **解决办法**：在文档打开阶段（`DocumentModel.init` 或 `buildColumns`）同步读取前 32 行样本一次性完成初始列宽计算，彻底移除 `loadNext()` 中的异步改宽逻辑（`initialFitsRemaining`）；在 `sizeToFitWidthOfColumn` 中优先直接读取 `RowCache` 已缓存的当前可见行数据同步返回精确列宽。
- [ ] **修复搜索高亮硬编码字体颜色导致选中行反白失效的问题**
  - **问题**：`TableCells.swift` 的 `highlighted()` 与 `DataTable.swift` 的 `render()` 将文本颜色硬编码为 `NSColor.labelColor` 与 `.black`，破坏了 `NSTableView` 在选中行（深蓝/强调色高亮背景）下自动将文字反白（`alternateSelectedControlTextColor`）的原生机制。
  - **解决办法**：在 `highlighted()` 中不对未命中的普通字符设置显式 `.foregroundColor`（仅对命中子串区间设置黄色底色 `.backgroundColor: .findHighlightColor` 与深色前景 `.foregroundColor: .black`）；对于非文本列的整格命中（`MatchCellView.wholeCellMatch`），根据 `backgroundStyle == .emphasized` 动态切换文本与高亮胶囊配色。
- [ ] **优化 `TextChoicesModel` 无条件 200ms 延迟与翻页闪白问题**
  - **问题**：`TextChoicesModel.load()` 在每次调用开头都无条件执行 `page = nil` 与 `Task.sleep(for: .milliseconds(200))`，导致首次点开文本列漏斗弹窗或点击 `Next Page` / `First Page` 翻页时也会白等 200ms 并清空列表造成闪烁。
  - **解决办法**：为 `TextChoicesModel.load()` 增加 `debounce: Bool` 参数，仅在 `query` 搜索词输入变化时传入 `debounce: true`（首次打开与翻页传入 `false` 立即执行）；加载新页期间保留旧 `page` 数据展示，待新页返回后再原子替换。
- [ ] **修复 `HistogramView` 鼠标移出后全局光标卡死（Cursor Leak）问题**
  - **问题**：`HistogramView.swift` 在 `mouseMoved` 中直接调用 `NSCursor.crosshair.set()` / `resizeLeftRight.set()` / `openHand.set()`，但 `NSTrackingArea` 未注册 `.mouseEnteredAndExited` 且未实现 `mouseExited` 恢复默认箭头光标，同时使用了 `.activeAlways`，导致鼠标移出直方图区域甚至窗口非激活时全局光标仍卡在十字准星或双向箭头状态。
  - **解决办法**：将 `NSTrackingArea` 选项改为 `[.mouseMoved, .mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect]`，新增 `override func mouseExited(with event: NSEvent) { NSCursor.arrow.set() }`，并在 `dismantleNSView` 时重置光标。
- [ ] **补齐 `ColumnFilterView` 与 `ExportModel` 异步任务的取消传递及 `CancellationError` 兜底过滤**
  - **问题**：`ColumnFilterView.load()` 使用非结构化的 `Task.detached` 调用 `doc.filterSpecWithBins(...)`，在弹窗关闭或切换分箱/比例尺导致 `.task(id:)` 取消时，既未取消后台任务也未通知 Rust 端中断；`ExportModel.cancel()` 仅调用了 `job?.cancel()` 而未调用 `task?.cancel()`；部分 `reportError` 未统一过滤 `Swift.CancellationError`。
  - **解决办法**：结合 1.7 的可取消过滤会话句柄，在 `ColumnFilterView.load()` 中使用 `withTaskCancellationHandler` 桥接 `.task(id:)` 的取消事件并在 `onCancel` 中调用 Rust 会话的 `cancel()`；在 `ExportModel.cancel()` 中同步调用 `task?.cancel()`；在所有 `reportError` 入口统一增加 `if error is CancellationError || (error as? GewuError) == .Cancelled { return }`。
- [ ] **修复超大行块（>8MB）拒绝入库引发的后台无限重拉死循环与 `failedVersion` 整表锁死**
  - **问题**：当 64 行块包含超长文本（如大 JSON / Base64）总字节超过 `RowCache` 的 8MB 上限时，`cache.insert` 返回 `false` 拒绝入库；紧接着行号槽重绘（`gutter.needsDisplay = true`）或滚动 1px 再次调用 `block(for:)` 发现缓存仍为 `nil`，会立即将该 `Key` 重新加入 `pending` 队列并触发 `loadNext()`，造成无法停止的高频死循环；此外，一旦任意块读取抛错，`self.failedVersion = version` 会将整个视图版本永久锁死在 `"…"` 状态。
  - **解决办法**：在 `RowCache` 中增设单槽位临时缓存 `private var oversized: (key: Key, block: RowBlock)?` 兜底保存最近一次超出预算的块，防止同一视口位置重复入队；将 `DataTableController.failedVersion` 改为按块记录 `private var failedKeys: Set<RowCache.Key> = []`（在视图版本切换时清空），避免单个块异常锁死整张表。
- [ ] **优化 `RowCache` 与 `ScatterChart` 热路径性能（消除每帧 8,192 次 `ChartProxy.position` 投影计算）**
  - **问题**：`RowCache.block(for:)` 在每次单元格渲染和行号槽（`RowGutterView.draw`）每帧滚动重绘时都会对 `order` 数组执行 `order.removeAll { $0 == key }` 的 `O(N)` 线性扫描与搬移；同时 `DataTableController.pending` 采用 FIFO（`removeFirst()`）导致快速滚动时先加载已滚过的历史块；`ScatterChart` 在 `onContinuousHover` 每帧都在主线程对 4,096 个点调用 `proxy.position(forX:)` / `proxy.position(forY:)`（单帧高达 8,192 次坐标投影计算）并无条件写入 `@Binding var hovered` 触发全量重绘。
  - **解决办法**：在 `RowCache` 中用单调递增访问计数（或双向链表节点）替代 `Array.removeAll` 实现 `O(1)` 命中更新，并在 `RowGutterView.draw` 中按 64 行块复用同一次 `block` 查询；将 `DataTableController.pending` 改为 LIFO（优先弹出当前处于 `visibleRect` 内的块）；在 `ScatterChart.nearest(to:proxy:)` 中改为对鼠标当前像素坐标调用 1 次 `proxy.value(atX:as:)` / `proxy.value(atY:as:)`（或结合 `proxy.plotSize` 按轴极值做简单线性/对数归一化），仅当 `nearest?.sourceRow != hovered?.sourceRow` 时才更新 `hovered`，并将高亮悬停点拆分为独立的叠加 `PointMark`。
- [ ] **收敛散落的 `UserDefaults` 字符串常量并补齐 `ShowColumnInspector` 持久化写入**
  - **问题**：`WorkbookModel.swift` 仅在初始化时读取 `UserDefaults.standard.bool(forKey: "ShowColumnInspector")`，但切换 `showsInspector` 时从未写回 `UserDefaults`；同时 `"TableFontSize"`、`"RowNumberBase"` 等近 10 个偏好设置 Key 以裸字符串字面量散落在 8 个文件中。
  - **解决办法**：新建统一的 `AppPreferences` 集中定义所有 `UserDefaults` Key 常量与初始默认值（通过 `UserDefaults.standard.register(defaults:)` 统一注册），并在 `WorkbookModel.showsInspector` 的 `didSet` 中写回 `UserDefaults`。
- [ ] **修复 AppKit 弱引用代理、视图挂载顺序及几何/类型边界防御隐患**
  - **问题**：
    1. `ExportModel.swift` 将局部变量 `let selection = ExportSelection()` 赋值给 `weak` 属性 `NSSavePanel.delegate`，当默认格式非 Parquet 时仅靠闭包捕获维持生命周期；
    2. `RowGutterView.init(table:)` 依赖传入的 `table` 已经挂载到 `enclosingScrollView` 才能注册滚动通知；
    3. `ExportFileFormat.swift` 对 `.xlsx` 使用了强制解包 `UTType("org.openxmlformats.spreadsheetml.sheet")!`；
    4. `HistogramView.snappedValue` 未校验 `plot.width > 0`（宽度为 0 时除零产生 `NaN`），`MatchCellView.draw` 在列宽 `< 4pt` 时计算 `bounds.width - 4` 产生负宽度 `NSRect`；
    5. `FileInfoView` 与 `ColumnInfoView` 中 `ForEach` 使用了不稳定的 `id: \.offset`。
  - **解决办法**：
    1. 在 `ExportModel` 中新增实例属性 `private var selection: ExportSelection?` 显式强引用持有代理直至 Sheet 关闭；在 `RowGutterView` 中将滚动监听注册移至 `viewDidMoveToSuperview()`；
    2. 在 `ExportFileFormat.swift` 中改为非可选的 `UTType(importedAs: "org.openxmlformats.spreadsheetml.sheet", conformingTo: .data)`；
    3. 在 `HistogramView.snappedValue` 开头增加 `guard !counts.isEmpty, plot.width > 0 else { return lo }`，在 `MatchCellView.draw` 中使用 `max(0, min(text.width + 8, bounds.width - 4))`，在 `DataTable` 单元格渲染与选中行映射中补齐 `offset >= 0 && offset < count` 下界断言；
    4. 为 `FileInfoView` 的详情项与 `ColumnInfoView` 的列表项提供稳定 `id`（或遵循 `Identifiable`）。

### 1.7 前后端 FFI 边界与引擎协同优化 (FFI & Core Synergy)

当前问题：虽然前后端在快照隔离（Snapshot Isolation）与精确边界对象（`Bound`）上设计扎实，但在表格数据块拉取粒度、Actor 协作线程池阻塞、Facet 过滤行集复用、同列多模块扫描、日期坐标跨边界传输及 UniFFI 历史接口收敛上存在明显的不合理开销。

- [ ] **将 `TableRowReader` 同步 FFI 调用移出 Swift Actor 协作线程池**
  - **问题**：`RowCache.swift` 中的 `TableRowReader` 声明为 Swift `actor`，在其隔离上下文中直接调用同步阻塞的 UniFFI 方法 `document.rows(...)`，会直接占用 Swift Concurrency 全局有限的协作线程池（Cooperative Thread Pool，线程数等于 CPU 核心数），在高负载或大文本格式化时易导致其他 UI 协程饥饿。
  - **解决办法**：将 `TableRowReader` 内部改为持有一个专用的串行后台队列 `DispatchQueue(label: "gewu.table.reader", qos: .userInitiated)`，通过 `withCheckedThrowingContinuation` 在该队列上检查任务取消并执行 `document.rows(...)`，释放 Swift 协作线程池。
- [ ] **合并可视列批量拉取，消除“单列 × 64 行”碎片化 FFI 调用与重复计算**
  - **问题**：`GewuDocument.rows(version, start, count, columns)` 原生支持一次传入多列，但 Swift 端 `RowCache` 按单列拆分请求，导致同屏 15 列需串行发起 16 次异步 FFI 调用，每次重复克隆相同的 64 个 `source_rows: Vec<u64>`；且在存在搜索词时，Rust 端每次调用 `rows()` 都会重新构造 `UInt32Array`、执行 Arrow `take` 拷贝并重新编译正则表达式。
  - **解决办法**：Swift 端在 `fillVisiblePlaceholders` / `loadNext` 中将同一行块起点（`start`）下所有缺失的当前可视列合并为一次 `doc.rows(version, start, count, columns)` 批量请求，再按列拆入 `RowCache`；Rust 端缓存或移除 `rows()` 内临时的正则重编译与 `take` 拷贝（结合 1.1 查找匹配索引直接标记命中单元格）。
- [ ] **优化列漏斗（Facet）行集复用，补齐取消令牌（Cancellation Token）**
  - **问题**：当点开自身尚未设置过滤条件的列漏斗弹窗时，其 Facet 行集完全等价于当前视图 `current_view.rows`，但 `RowScope::Facet` 仍会无条件调用 `self.select(...)` 从头重扫全表其他列的过滤条件；在数值列漏斗弹窗中切换 `Maximum bins` 或 `Linear / Log10` 比例尺时每次都会重跑全表过滤；且 `RowScope::Facet` 硬编码了 `CancelToken::never()`。
  - **解决办法**：
    1. 在 `Document::rows_in(RowScope::Facet { state, except })` 中，若 `!state.filters.iter().any(|f| f.column() == except)` 且 `state` 与当前已发布视图状态一致，直接复用 `self.views.current().rows.clone()`（`O(1)` 零扫描）；
    2. 新增可取消的 `FilterSession` FFI 对象（类似 `TextChoices`），首次打开漏斗时捕获一次 Facet `Selection`，弹窗内切换 `bins` 或 `scale` 时直接在已捕获的 `Selection` 上重算直方图，并提供 `.cancel()` 方法供 Swift `.task(id:)` 取消时调用。
- [ ] **合并 Inspector 同列多 Section 扫描，消除对同一列内存的 4~5 次重复遍历**
  - **问题**：`InspectorModel` 当前按 `Overview` → `Statistics` → `Distribution` → `Values` 串行发起独立 FFI 任务：其中 `Overview` 扫一遍算空值，`Statistics` 扫一遍算极值/均值/标准差/NaN/Inf，`Distribution` 内部又重扫一遍算极值/空值/NaN/Inf 再做分箱。
  - **解决办法**：在 Rust 端为数值列的 `ColumnAnalysis` 提供共享的列统计快照（或合并 `Overview` + `Statistics` + `Distribution` 的基础一遍扫描，直接将第一遍扫描得出的 `min / max / null_count / nan_count / infinite_count` 传给直方图分箱器，省去直方图内部重复的极值扫描 pass）。
- [ ] **将查找列定位（`moveToMatch`）收归 Rust，消除 Swift 端循环格式化整行字符串的浪费**
  - **问题**：`DataTable.swift` 的 `moveToMatch` 为了获知命中行内哪一列匹配，在 Swift 端按 64 列步长循环调用 `doc.rows()`，迫使 Rust 格式化整行单元格字符串跨 FFI 传回后又将 `block.cells` 全部丢弃，仅读取 `block.matched`。
  - **解决办法**：在 Rust 端提供直接查询匹配单元格列坐标的轻量接口（结合 1.1 的查找索引，直接返回 `(display_row, column)`），彻底移除 Swift 端 `moveToMatch` 中的 `stride(from: 0, to: columns, by: 64)` 循环。
- [ ] **散点图日期/时间轴直接传输归一化时间戳，消除 Swift 端 `ISO8601DateFormatter` 逐点字符串反解**
  - **问题**：`scatter.rs` 将时间列转为无单位标识的物理整数 `f64` 与格式化字符串 `x_label / y_label`，导致 Swift 端在 `ScatterDates.make` 中创建两个 `ISO8601DateFormatter`、手动拼接 `"Z"` 时区并对最多 `4,096 × 2 = 8,192` 个字符串逐个反向解析回 `Foundation.Date`（且一旦任一点含纳秒或格式不兼容便整轴静默退化为裸数字轴）。
  - **解决办法**：在 `scatter.rs` 中，当轴列为 `Date32`、`Date64` 或 `Timestamp(unit, _)` 时，将 `ScatterPoint.x` / `y` 统一按对应的 Arrow 时间单位换算为 Unix Epoch 秒数（`f64`）；Swift 端直接通过 `Date(timeIntervalSince1970: point.x)` `O(1)` 构造日期坐标，删除 `ScatterDates.make` 中的 `ISO8601DateFormatter` 字符串解析逻辑。
- [ ] **低基数文本列（`DataKind::Text`）过滤候选复用 `ValueIndex`**
  - **问题**：`Document` 已具备基数 $\le 100,000$ 的 `ValueIndex` 懒加载索引，但 `filter_spec_with_scale` 对所有 `DataKind::Text` 列一律走 `TextChoices::page_checked` 全表扫描；导致即使是仅有几个枚举值的千万行字符串列，每次打开弹窗、翻页或输入搜索字符都要从第 `0` 行到第 `N` 行重扫整列。
  - **解决办法**：在 `prepare_text_choices` 中，若该文本列已构建（或可构建）`ValueIndex`（基数 $\le \text{INDEX\_LIMIT}$），直接在 `ValueIndex` 上执行一次 `counts_all(&rows)` 缓存有序候选列表；后续 `TextChoices::page_for_generation` 直接在该有序字典列表上做二分定位与子串过滤，使翻页与搜索耗时从 `O(N_rows)` 降至 `O(K_distinct)`。
- [ ] **清理 UniFFI 边界残留的冗余平行接口与死数据结构**
  - **问题**：演进过程中遗留了多套“新旧并存”的 FFI 接口与未使用的类型，扩大了生成的绑定体积与维护负担。
  - **解决办法**：移除前端零调用的 `column_summary`（及 `ColumnSummary` 记录）与 `view_histogram`；利用 UniFFI 的 `#[uniffi(default = ...)]` 默认参数机制，将 `filter_spec` 与 `filter_spec_with_bins`、`prepare_analysis` 与 `prepare_analysis_with_bins`、`open` / `open_sheet` 与 `open_with_options`、`Export::write` 与 `write_with_options`、`TextChoices::page` 与 `page_for_generation` 分别收敛为单一规范方法，并同步更新 Rust 侧 FFI 测试。

### 1.8 Rust 核心引擎性能与健壮性优化 (Rust Core Engine)

当前问题：Rust 后端（`gewu-core`）在低基数索引构建与取消检查、大文件批次内存合并、CSV 多遍扫描、分隔符文件后缀嗅探、Parquet 字典列支持、XLSX 错误单元格容错及全表行号分配上存在若干深层瓶颈与兼容性限制。

- [ ] **消除低基数列 `has_at_most` 与 `ValueIndex::build` 的二次重复全表扫描，并补齐 `CancelToken` 检查**
  - **问题**：`document/filter.rs` 在首次打开列漏斗时先调 `value_index::has_at_most(array, 500)` 用 `RowConverter` 扫完千万行判断基数并丢弃哈希集合，紧接着在下一行进入 `FilterEditorKind::Choices` 时又调 `ValueIndex::build(array, 100_000)` 对同一列重跑一遍完全相同的 `RowConverter` 全列扫描；且 `ValueIndex::build` 在按 `CHUNK`（4,096 行）循环构建哈希表时未检查 `CancelToken`，在千万行大表上关闭弹窗后后台线程无法提前中止。
  - **解决办法**：在 `ValueIndex::build(array, limit, cancel: &CancelToken)` 的每个 `CHUNK` 循环开头增加 `cancel.check()?`；在 `document/filter.rs` 判断 `low_cardinality` 时，直接调用 `self.index(column, &cancel)?`（内部调用一次 `ValueIndex::build` 并缓存到 `self.indexes[column]`），然后检查返回的 `index.len() <= CHOICE_LIMIT`，一举消除二次扫描并支持随时取消。
- [ ] **优化 `collect_batches` 强制 `concat_batches` 导致的 2× 内存峰值与 2 GiB 字符串偏移溢出**
  - **问题**：`file/mod.rs` 在加载 Parquet / CSV 时先将全部 `RecordBatch` 存入 `kept: Vec<RecordBatch>`，再调 `arrow::compute::concat_batches` 分配一整块连续内存并全量拷贝，导致打开文件瞬间内存峰值翻倍（2× RAM）；且当 `Utf8` / `Binary` 列总字节数超过 2 GiB（`i32::MAX`）时会因 32 位偏移量溢出直接报错失败。
  - **解决办法**：
    1. 在按列合并时，将 `kept: Vec<RecordBatch>` 拆解为按列的 `Vec<Vec<ArrayRef>>`，每合并完一列（`arrow::compute::concat`）立即释放该列原有的分块 `Vec<ArrayRef>`，将打开文件时的额外内存峰值从“整表 100%（2× RAM）”降至“单列大小（$\approx 1 + 1/N_{\text{cols}}$）”；
    2. 在合并 `Utf8` / `Binary` 列前统计总字节数，若超过 `i32::MAX`（2 GiB），自动先将分块零拷贝/轻量转换为 `LargeUtf8` / `LargeBinary`（64 位偏移量）再执行 `concat`，彻底解决超长文本表的 2 GiB 溢出崩溃。
- [ ] **减少 CSV / TSV 打开时的 3 遍全文件顺序解析开销，并支持常见文本后缀与分隔符嗅探**
  - **问题**：`file/delimited.rs` 打开含浮点列的 CSV 时，第 1 遍用 `infer_schema(&mut file, None)` 扫完整个文件推断类型，第 2 遍在 `preserve_integer_precision` 中用 `csv::Reader` 重新逐行解析整个文件检查大整数精度（甚至所有待查列已标记 `preserve = true` 后仍不提前退出），第 3 遍再重新解析整个文件生成 Arrow 批次；此外 `FileType::detect` 仅硬编码匹配 `.parquet` / `.csv` / `.tsv` / `.xlsx` 后缀，拒绝打开 `.txt` / `.tab` 等合法分隔符数据文件。
  - **解决办法**：
    1. 在 `preserve_integer_precision` 中，将 `floating` 维护为待检查列集合，一旦某列检测到 $> 2^{53}$ 的整数失真即从待检集合移除，当集合为空时立即 `break` 提前终止文件扫描；且仅对不含小数点/指数符号且长度 $\ge 16$ 字节的字段调用 `parse::<i64>()`，跳过普通小数的解析开销；
    2. 支持前 N 行采样推断（或单遍同时完成 Schema 推断与大整数精度检测），将最多 3 遍全文件磁盘读取收敛为 1~2 遍；
    3. 在 `FileType::detect` 中扩展支持 `.tab`（映射为 `Tsv`）以及基于文件前几行 UTF-8 文本的分隔符嗅探（自动识别制表符或逗号分隔的 `.txt` / 无后缀文件）。
- [ ] **补齐 Parquet 字典编码列（`DataType::Dictionary`）的统计、直方图、散点图与长度分析能力**
  - **问题**：Parquet 文件极其普遍地使用 `Dictionary(Int32, ...)` 存储字符串甚至数值/日期列，但当前 `lengths::unit` 漏判了 `Dictionary` 包装的字符串列（导致侧边栏缺失 `Lengths` 面板），且 `Column::capabilities` 与 `Numeric::supports` 对所有 `Dictionary` 列一刀切禁用了 `histogram`、`scatter`、`logarithmic` 与 `Statistics`。
  - **解决办法**：
    1. 在 `lengths::unit` 中递归解包 `DataType::Dictionary(_, values)`，并在 `lengths::analyze` 中通过 `downcast_dictionary_array!` 先预计算字典 `values()` 的长度表（`O(K_dict)`），再按 `keys()` 汇总统计与直方图；
    2. 在 `Column::capabilities`、`Numeric::new`、`histogram` 与 `scatter` 中支持 `DataType::Dictionary(_, values)`（通过字典 `keys` 索引底层数值/时间 `values` 数组），使字典编码的数值/日期列拥有与普通列完全一致的统计、直方图与散点图能力。
- [ ] **改进 XLSX 公式错误单元格（`#N/A`、`#DIV/0!`）的容错处理并减少 `Range` 遍历次数**
  - **问题**：`file/xlsx.rs` 当前只要在工作表中扫到哪怕 1 个 `Data::Error`（如业务表中极常见的 `VLOOKUP` `#N/A`）就拒绝打开整张表；且对 `calamine::Range` 进行了 3 遍遍历（扫错误、推断列类型、构建 Array）。
  - **解决办法**：移除 `xlsx::read` 开头拒绝整表的 `Data::Error` 拦截循环；在 `column_array` 中将 `Data::Error(e)` 视为缺失值（`Null`）以保持数值/时间列的原生类型推断（或在文本列中保留错误字面量如 `"#N/A"`），确保含零星公式错误的真实 Excel 报表可正常打开查看。
- [ ] **消除未过滤视图与中位数计算（`median`）中冗余的全表行号数组分配**
  - **问题**：`ViewEngine::new` 与无过滤 `select()` 会分配长达 `n_rows` 的 `[0, 1, ..., n_rows - 1]` 恒等映射 `Vec<u32>`（千万行级白占数十至上百 MB 内存）；`analysis/median.rs` 对原生数值列计算中位数时，每次都分配完整 `ids: Vec<u32>` 并通过动态分发比较器 `DynComparator`（虚函数间接寻址）做原地选择。
  - **解决办法**：
    1. 将 `View.rows` 从 `Arc<Vec<u32>>` 升级为复用 `Selection` 枚举（`Selection::All(usize)` 表示未过滤未排序的恒等顺序，`Selection::Rows(Arc<Vec<u32>>)` 表示过滤或排序后的显式行号），使初始打开与清空过滤后的视图达到 **0 字节**行号内存占用；
    2. 在 `analysis/median.rs` 中，对原生数值与时间类型（`Int*`、`UInt*`、`Float*`、`Decimal*`、`Date*`、`Timestamp*`）直接收集紧凑的原生值 `Vec<T::Native>` 调用标准库 `select_nth_unstable_by`（内联比较指令），消除 `DynComparator` 虚函数调用与跨行随机内存访问开销。

---

## 2. 文档与开源展示改进 (Documentation & Showcase)

当前 README 严谨克制，但作为开源 macOS 桌面应用，缺乏视觉呈现，核心性能与差异化卖点表达过于平淡，更像一份“技术说明书”而非“产品主页”。

### 2.1 视觉门面建设 (Hero Preview)

- [ ] **准备代表性真实大数据集**
  - 选用经典且直观的公开数据集（推荐：单月 290 万行的 NYC Taxi Parquet 数据，或包含嵌套列表的 GitHub Repositories 表）。
- [ ] **截取高质量实机图片与动图**
  - **主界面截图（Dark Mode）**：展示几百万行数据秒开状态、激活的列过滤器、展开的 Inspector 统计面板与车费分布直方图（Histogram）或行程散点图（Scatter Plot）。
  - **交互短动图 / 视频（8~10 秒）**：展示拖入 Parquet 文件瞬间渲染、点击列即时重绘直方图、直接拖动直方图滑块调整数字范围过滤的丝滑过程。
- [ ] **更新 README 顶部结构**
  - 在标题下方直接放置 Hero 截图，让访问者在 3 秒内看懂软件形态与交互质感。

### 2.2 定位与差异化价值提炼 (Why Gewu)

- [ ] **强化解决的核心痛点**
  - 明确表述：“在把数百万行 Parquet/CSV 喂给 Python / Jupyter 之前，双击即可无感秒开，即时查看数据分布与进行预筛选”。
- [ ] **阐明架构优势（对抗传统 Electron 工具）**
  - 突出 **Native Swift (AppKit) + Rust (Apache Arrow)** 组合，强调 **零 Electron 膨胀、极低常驻内存消耗、ProMotion 原生流畅滚动**。
- [ ] **补充同类生态定位（与 DuckDB 形成互补）**
  - 说明 Gewu 是数据工程师与分析师日常配合 DuckDB / Polars 使用的轻量、免写 SQL 视觉探查伴侣。

### 2.3 特性叙事升级 (Features Showcase)

- [ ] **重构特性列表（从扁平描述改为亮点聚焦）**
  - **Native Mac Performance**：基于 Arrow 内存引擎，千万级虚拟滚动、多条件复合过滤无延迟。
  - **Instant Column Profiling**：侧边栏常驻中位数、极值、空值率与频次统计。
  - **Interactive Histograms & Scatter**：支持在直方图上直接拖动双把手滑块框选过滤范围；支持采样散点图。
  - **Rich Format & Nested Data**：原生支持 Parquet 嵌套结构（Structs & Lists）、CSV/TSV 以及多工作表 XLSX。
  - **Safe & Non-Destructive**：纯只读机制，彻底杜绝误修改源数据风险。

### 2.4 开发者与自动化友好 (CLI & Ecosystem)

- [ ] **提升 CLI 曝光度**
  - 在特性或快速上手部分增加独立说明：Gewu 附带高性能独立命令行工具（`gewu`），输出结构化 JSON，专为脚本自动化与 AI Coding Agent 设计。
- [ ] **规范与认证状态透明化**
  - 保持并优化 macOS Quarantine（未公证）处理说明，未来条件成熟时规划 Apple Developer ID 签名与 Homebrew Cask 分发。
