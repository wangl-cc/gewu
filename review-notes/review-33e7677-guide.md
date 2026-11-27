# gpui 版代码审查：以《gewu 工程指南》为标准

- 日期：2026-11-27
- 代码：`native-gpui` 分支 **33e7677**（路径省略 `crates/` 前缀）
- 标准：`/mnt/project-files/style-guide-draft.md`（下文条款编号均指它）
- 标记：**已核实** = 代码事实，带 `文件:行`；**判断** = 设计评价或建议，未经实验验证。
- 待定条款（T3、E3 范围、V5、U4 命令表、G1、G3、G4、M1、M2 场景测试、M7、M8）下的问题只在第五节报现象，不算违规。

## 结论摘要

1. 最大的问题是 **`TableView` 实际上是“文档内容视图”**：表格交互、查询调度、复制、查找宿主、标题栏、工具栏、状态栏、Inspector、导出、列管理、排序面板、右键菜单都挂在它身上，方法分散在 13 个文件的 `impl TableView` 里。交互规则因此只能开窗口测（U1、U2、G2、D1、S1、Q1）。
2. 第二是 **文档级状态放错了所有者**：切换工作表或改导入选项会新建一个 `TableView`，再把导出控制器和 Inspector 设置“继承”过去，`Document` 还直接写表格的字段（S1、P3）。
3. 第三是 **“哪个结果有效”的调度规则各写各的**，最重的是 `FindController`：16 个字段，几组平行状态靠手动同步；`gewu-app` 的 Find 只提供了 `step`（U1、S2、C1、P1）。
4. 平台隔离有三处突破，`platform/update.rs` 实际是一个完整的应用功能，还反向依赖 `shell`（L5、D1）。
5. 文件格式知识（扩展名、能否设表头、显示名）在 UI 里重写了四五份，名字还不一致（P1、L2）。
6. 错误在 UI 状态里普遍先转成字符串（E2），错误文案有 7 个分支直接拼 Core 的 `Display`（E3）。

好的部分也很明确：`QuerySession` 的模型写法、渲染路径单点提交（S3）、`Drawer` / `Load` / `Showing` / `ExportController` 这些枚举状态（S2）、按处理器推导菜单可用状态（S4）、`unsafe` 只在 `platform::macos`。改进方向是把这些已有的好做法推广到其余部分，而不是另起炉灶。

---

## 一、架构级问题

### A1 `TableView` 承担了整份文档的职责（U1、U2、G2、D1、S1、Q1）

**已核实**
- `table/mod.rs:48-76` 有 26 个字段，`table/controls/mod.rs:24-33` 的 `Controls` 再加 7 个（抽屉、菜单、待隐藏列、面板焦点、Inspector、散点标记、导出）。
- 13 个文件写 `impl TableView`：`table/{mod, render, input, frame, header, layout, query}.rs` 和 `table/controls/{mod, columns, sorting, export, menus, status}.rs`；其中 8 个用 `use super::*;` 拿到全部私有字段。
- 职责清单（都在 `TableView` 上）：网格交互（选区、滚动、拖动、悬停，`input.rs`）；查询调度（`query.rs:407-452`）；复制（`mod.rs:352-395`）；查找宿主（`mod.rs:168-179`、`198-200`）；窗口标题栏（`render.rs:6-70` `document_titlebar`）；工具栏和 Inspector 开关（`render.rs:262-296`）；空结果提示（`render.rs:200-258`）；状态栏（`controls/status.rs`）；Inspector 宿主（`controls/mod.rs:175-246`）；导出（`controls/export.rs`）；列管理（`controls/columns.rs`，502 行）；排序面板；右键菜单和单元格详情（`controls/menus.rs`）。
- 交互规则没有不依赖窗口的模型：`table/` 下 `edge_tests`（19）、`gesture_tests`（9）、`workflow_tests`（4）、`controls/tests`（17）都通过 `ui_fixture.rs:15` 的 `TableWindow` 开测试窗口。纯模型测试只有 `scroll_tests`、`selection_tests`、`layout_tests`、`find_style_tests`、`data_tests`。
- `input.rs:12-70` `mouse_down` 在一个函数里做命中测试并直接改 `drag`、选区、排序、筛选面板，`input.rs:171-214` `extend_drag` 自己算一份 `Layout` 再写回 `scroll` 和 `horizontal`。

**为什么是问题（判断）**：指南 2.7 要求 Entity 只当外壳，判断逻辑放进它持有的纯模型。现在的结构让“改一个交互”需要理解 30 多个字段之间的关系，任何子模块都能改任何字段，S1 的“只有一个所有者”在 Entity 内部失效。指南 D2 的历史反例（`find.rs` 写 `impl TableView`）现在以更大的规模存在于 `controls/`（见第六节对 D2 措辞的反馈）。

**改进方向（判断）**
1. 先做 A2，把文档级的东西移出去，`TableView` 只剩网格。
2. 抽出不依赖 gpui 的 `GridModel`：持有 `Selection`、`VerticalScroll`、`HorizontalScroll`、`Drag`、悬停；方法形如 `pointer_down(hit, modifiers, clicks) -> GridEffect`，`GridEffect` 枚举“循环排序某列”“打开某列筛选”“显示单元格详情”等。命中测试由 `Layout` 给出（见 B4）。现在 `scroll.rs`、`selection.rs` 已经是这种写法的一部分，扩大即可。
3. `TableView` 只负责：把 gpui 事件翻译成 `GridModel` 调用、执行返回的副作用、`notify`。`edge_tests` / `gesture_tests` 里大部分规则随之能改成纯测试，窗口测试只留接线（Q1）。

### A2 文档级状态放在了表格里，换表时靠复制（S1、S4、P3）

**已核实**
- `shell/document.rs:214-224`：每次 `install` 都新建 `TableView`，然后直接写它的 `document_title`、`worksheet_tabs` 字段（217-218），有旧表时调用 `inherit_document_state`（`table/controls/mod.rs:181-197`）复制导出实体、按旧设置重建 Inspector。
- `table/controls/export.rs:188-189` 的注释：“Mounting the same entity in the replacement table keeps its status and progress subscribed”，这是为所有权放错位置写的补丁。
- `Document` 读表格内部：`document.rs:240-242` 读 `data.query()`、`data.queries().requested()`；`document.rs:412` 读 `data.snapshot().dataset()`。
- 窗口标题栏由表格渲染：`render.rs:262-264` 根据 `document_title` 决定画 `document_titlebar` 还是普通工具栏。
- 两份 Inspector 可见性：`Inspector.visible` 和 `Preferences.show_inspector`，由 `controls/mod.rs:222-233` 手动同步（S4 没写清角色）。

**为什么是问题（判断）**：Inspector、导出进度、标题、工作表标签、工具栏是“这份文档”的状态，生命周期比某一次导入长。放在 `TableView` 里，换表就必须逐项搬运，漏一项就是 bug，而且新增一种文档级状态时没人会想起要加进 `inherit_document_state`。

**改进方向（判断）**：`Document` 持有 Inspector、`ExportController`、工作表标签，渲染标题栏、工具栏和状态栏外框；`TableView` 只渲染网格和它自己的弹层。`Document` 需要的查询和快照信息通过 `TableView` 的方法或事件拿，不读 `data` 字段。`Preferences.show_inspector` 作为“下次打开时的初始值”，在注释里写清它和 `Inspector.visible` 的关系，或者只保留一份。

### A3 调度规则写在 Entity 里，而且有四种写法（U1、S2、C1、P1）

**已核实**
- `table/find.rs:71-88` `FindController` 有 16 个字段。平行状态：
  - `cursor` 和 `message: FindMessage::Location { .. }` 都在描述当前命中（`find.rs:204-209` 同时写两者，`457-458` 同时清）；
  - `highlight_failed` 和 `FindMessage::Error` 同时表达“失败”（`301-302`、`374-376`）；
  - `navigation`、`queued`、`counter`、`reading` 四个任务相关字段；
  - `invalidate`（`100-110`）手动重置 9 个字段，这正是 S2 说的“几个必须一起变的字段”。
- 调度规则全在外壳里：导航排队合并（`423-427`、`465-468`）；计数器按起点加位移推算当前序号（`34-51`、`450-454`）；高亮分批读取的规划（`319-382`）；失败后不再重试。纯测试只有 1 个（`find_tests.rs`，测 `Highlights`）。`gewu-app/src/find.rs` 只提供 `step`。
- 同类问题的其他写法：
  - `shell/document.rs:138-197` `start_pending`：“只开一个读取、只保留最新导入”，写在 Entity 里，靠 `Load::Pending(latest) if *latest == import`（185）判断；
  - `panels/filter/mod.rs:70-86` `FilterEditor` 用 `preview`、`error`、`load` 三个字段表达“加载中 / 就绪 / 失败”，而 `panels/inspector/mod.rs:37-43` 的 Inspector 用 `enum Showing { Empty, Loading, Column, Chart, Failed(Error) }` 表达同一件事；
  - `platform/clipboard.rs:4-36` 用全局代数加系统粘贴板 changeCount 判断复制结果是否过时。
- 对照：`gewu-app/src/query.rs` 的 `QuerySession` 是 U1 的正例，`table/query.rs:407-452` 的外壳也基本只做执行。

**改进方向（判断）**
1. Find 按 `QuerySession` 的形状重写成模型（放 gewu-app 或 UI crate 的纯模块）：一个 `enum FindState { Idle, Searching, At { hit, wrapped, count: CountState }, NoMatches, Failed(Error) }` 替代 `cursor` + `message` + `highlight_failed`；方法 `text_changed / step / scope_changed / step_finished / count_finished` 返回要执行的副作用（开导航任务、开计数任务、读高亮块、发 `CellMatch`）。外壳只执行。排队合并、计数推算都能不开窗口测。
2. `FilterEditor` 的加载状态改成和 Inspector 一样的枚举（P1：同类的事一种做法）。
3. `Document` 的导入调度抽成一个小模型（`ImportSession`），和 `QuerySession` 同形。
4. 剪贴板的代数机制可以保留（它防的是别的窗口和文本框写粘贴板），但在 `//!` 里写清它和 `copy_task` 丢弃取消的分工。

### A4 平台隔离被突破，`platform` 里装了应用功能（L5、D1、D2）

**已核实**
- Objective-C 调用出现在 `platform::macos` 之外：
  - `shell/picker.rs:5`、`11` 持有 `Retained<NSOpenPanel>`，`75`、`82` 直接调 `makeKeyAndOrderFront`、`performClose`；
  - `platform/clipboard.rs:57`、`66-68`、`85-110` 直接用 `NSPasteboard`（文件内自建 `mod macos`）；
  - `platform/recent.rs:5` 用 `NSDocumentController`。
  - `unsafe` 确实只在 `platform/macos/` 下（L5 这一半做到了）。
- `platform/update.rs` 是完整的应用功能：自己定义 actions（14-18）、`Updater` 实体（99）和 `Render`（288）、菜单（`menu`），并且 `use crate::shell::update_menus`（12），`platform` 反向依赖 `shell`。它用 gpui-kit 的 `Theme::global(cx).tokens.colors`（290），不走 `GewuPalette`（见 B8）。

**改进方向（判断）**：`NSOpenPanel` 包进 `platform::macos` 的一个小类型（`activate` / `dismiss`），`shell/picker.rs` 只持有它；粘贴板写入和最近文档的原生调用挪进 `platform::macos`。更新功能挪到 `shell/update.rs`（或独立的 `updates/` 模块），`platform` 只保留“调用 helper 进程并解析输出”。

### A5 文件格式知识散在 UI 里，名字也不一致（P1、L2、L1）

**已核实**
- Core 的扩展名识别是私有的：`gewu-core/src/file/mod.rs:62-86` `FileType::detect`。
- UI 各写一份：
  - `shell/mod.rs:286-292` `supports_header` 用扩展名列表 `["csv", "tsv", "xlsx"]`；
  - `shell/document.rs:60` 用它初始化 `header_available`，`206-209` 装载后又按 `dataset.format()` 重算一次；
  - `shell/document.rs:146` 单独判断 `"xlsx"`；
  - `table/controls/export.rs:201` 导出文件的扩展名取 `format.to_string().to_lowercase()`，把扩展名绑在了显示文字上。
- 显示名不一致：Core `Display` 给 `XLSX`（`gewu-core/src/file/mod.rs:96`），导出格式选择器直接用它（`export.rs:112`），错误文案也写 `XLSX`（`ui/errors.rs:68`）；信息窗口写 `Excel`（`shell/info.rs:45`）。

**改进方向（判断）**：Core 公开 `FileType::from_path`、`extension()`、`supports_header()` 这类通用语义（L2，写进 Core README 并测试）；UI 只在 `ui/messages.rs` 放一个 `format_name(FileType)`，导出选择器、信息窗口、错误文案都用它，不再用 Core 的 `Display` 当界面文字。

---

## 二、设计层问题

### B1 错误在状态里先转成字符串（E2、E1）

**已核实**，反例：
- `table/query.rs:5-9` `Notice::FilterFailed(String)`、`CopyFailed(String)`；
- `table/find.rs:63` `FindMessage::Error(String)`；
- `table/controls/export.rs:25-26` `Finished(String)`、`Failed(String)`（成功和取消的提示也是预先拼好的字符串）；
- `table/data.rs:44-51` `BlockText::Failed(SharedString)`、`80-87` 在缓存层就调 `errors::message`，`prefetch` 返回 `Option<SharedString>`（224-228）；
- `table/mod.rs:358` `copy_finished(failure: Option<String>)`；`platform/clipboard.rs:76` `Result<(), &'static str>`；
- `shell/picker.rs:16` `Result<Self, String>`；`shell/mod.rs:228` `report_window_error(message: String)`；
- `platform/update.rs:89` 起的 `Status::Failed(String)`，`Release::run` 返回 `Result<_, String>`。

正例：`Inspector` 的 `Showing::Failed(gewu_core::Error)`、`FilterEditor.error: Option<gewu_core::Error>`、`Document` 的 `Load::Failed(Import, Error)`、`QuerySession`。

**改进方向（判断）**：UI 自己的失败（粘贴板、文件选择器、helper 进程）定义小枚举，和 Core 的 `Error` 一起存进状态，文字只在 render 时选。`Notice` 单槽里各来源各清各的（`query.rs:385-391`、`mod.rs:358-366`）也随之能写成一个明确的规则。

### B2 错误文案的兜底分支比指南记录的多（E3）

**已核实**：`ui/errors.rs` 里直接用 Core `Display` 的分支有 7 个：76（`File(e) => e.to_string()`）、92（`Filter(e)`）、96（`Find(e)`）、103（`Analysis(e) => e.to_string()`）、113（`Export(e) => e.to_string()`）、114（`Column(e)`）、116（`Internal(e)`）。指南第六节只记了 3 个。同一句文案写两遍：“No matches” 在 `find.rs:216` 和 `486`。

### B3 命令散在三处，右键菜单是第二套命令系统（U4）

**已核实**
- 命令定义和绑定在三个地方：`actions.rs:3-16`（文档和表格级）、`shell/mod.rs:14` 起和 `135-145`（全局）、`platform/update.rs:14-18`（更新）。
- 右键菜单有自己的 `enum Command`（`controls/menus.rs:89`），可用状态在 `151-156` 另算一遍，执行走 `menu_command`（244-280）；同一个命令的 action 处理器和可用条件在 `render.rs:303-359`。例如“Filter Column…”既挂了 `EditFilter` action（用来显示快捷键），点击又走 `Command::Filter`。
- ✗ `panels/charts/scatter.rs:390-399` 在 `on_key_down` 里硬匹配 `left`、`right`、`enter`、`space`。
- ✗ 查找栏的“打开”有两个入口直接改字段：`find.rs:242-245`（输入框获得焦点）和 `547-555`（`capture_any_mouse_down`）。
- 指南已记录的 ✗：悬停状态在 `render.rs:126-136`、`input.rs:134-145`、`mod.rs:233-238` 三处清理。

**改进方向（判断）**：右键菜单项全部改成 action（菜单项的可用状态由 gpui 按处理器推导，和主菜单一样），`Command` 枚举删除。散点图的键盘操作定义成 action 并带 key context 绑定。U4 的命令表是否要建仍待定，但这里的证据支持建：三处定义加一套平行枚举，正是命令表要解决的问题。

### B4 表头命中几何不是唯一来源（G2、P1）

**已核实**：筛选漏斗的命中区在 `input.rs:47` 写成 `left + width - PAD - 14.`，绘制在 `paint.rs:276` 写成 `right - PAD - 9.`、宽 10；两个数各自维护。`filter_anchor`（`controls/mod.rs:346-358`）和 `hover_target`（`header.rs:33-102`）又各自从 `Layout` 推一遍列边界。`extend_drag`（`input.rs:171-214`）在 `commit_grid_layout`（`layout.rs:350-382`）之外，第二次把一份算出来的 `Layout` 写回 `scroll` 和 `horizontal`。

**改进方向（判断）**：`Layout` 提供 `header_part_at(x) -> Option<HeaderPart>`（`Title(col)` / `Funnel(col)` / `Divider(col)`）和对应的矩形，绘制和命中都用它。拖动边缘滚动在 A1 的 `GridModel` 里表达，不在输入处理器里另算布局。

### B5 `VisibleCol` 只引入了一半（T2、P1）

**已核实**：`table/columns.rs:99-107` 定义了 `VisibleCol`，但 `entry`、`set_width`、`get`、`width`、`Index<usize>`（138-181、232-238）都收裸 `usize`；`position()` 返回 `VisibleCol`，调用方马上 `.index()`（`data.rs:174`、`controls/mod.rs:353`、`layout.rs:33`）。这是 P1 说的“替换概念没替换完”。另外列名有两份：`GridColumn.label` 和 `info.name`（`columns.rs:93-97`），`frame.rs:103` 用 `label`，`header.rs:248` 和 `mod.rs:166` 用 `name`。指南已记录的 `Cell`、`Layout.columns` 三元组同属此类。

### B6 查找的匹配语义写在帧构建里（L4、U1）

**已核实**：`table/frame.rs:54-82` 决定每个可见单元格画哪些高亮段，包括“冷视口先在预览里证明字面匹配”“类型化相等可能匹配格式不同的值”这类 Find 语义。**判断**：这是 Find 模型的规则，应该由 Find（gewu-app 或 A3 的模型）给出“这个单元格的高亮段”，帧构建只搬运。

### B7 小的重复（P1）

**已核实**：排序箭头字符串在 `frame.rs:112-119` 和 `header.rs:317-324` 各写一次；换行字符集在 `data.rs:37` 和 `frame.rs:105` 各写一次；`ColumnWidth::Unmeasured` 的宽度 `110.` 是字面量（`columns.rs:87`）。

### B8 颜色有两套来源（V1、P1）

**已核实**：`shell/document.rs:284`、`401` 和 `platform/update.rs:290` 用 gpui-kit 的 `Theme::global(cx).tokens.colors`，其余界面用 `GewuPalette::get(cx)`。两者都是主题 token，但不是同一个模型；高对比度和配色切换只保证 `GewuPalette` 一侧（判断，未实测）。

### B9 `TableView` 为示例开了公开 API（M6、P1）

**已核实**：`lib.rs:17` `pub use table::TableView`；`table/mod.rs:136-143` 的 `pub fn from_dataset` 只被示例和示例测试调用（`examples/scroll.rs:460`、`examples/scatter_hover.rs:232`、`examples/scroll/tests.rs:37`），产品路径用 `TableView::new`（`document.rs:216`）；`from_dataset` 里还调用了 `follow_appearance`（141），在视图构造函数里给窗口注册外观观察者。`pub focus`、`pub fn scroll_to_row`、`pub fn filter_selected`、`pub fn clear_filters` 同理。**判断**：示例需要的入口单独放一个窄的 `examples` 支撑接口，产品类型保持 `pub(crate)`。

---

## 三、机械约定

- **M4 缺 `//!`**（已核实）：gewu-gpui 有 33 个非测试文件没有模块注释（`table/{render, input, frame, layout, paint, query, find_style}.rs`、`table/controls/{columns, export, menus, sorting, status}.rs`、`panels/` 下 9 个、`platform/{recent, storage}.rs`、`preferences/{file, window}.rs`、`shell/{assets, picker}.rs`、`theme/palettes/*`、`ui/messages.rs`、`lib.rs`）；`lib.rs` 也没有 code map。Core 67 个文件里 6 个没有。
- **M4 / M5 历史残留**（已核实）：`table/data.rs:1` “no FFI, no packing”；`table/data.rs:313` “as the Mac app copies”；`gewu-core/src/lib.rs:3` “The Mac app …”。
- **M3 文件内顺序**（已核实）：`table/mod.rs:30` 和 `402-430`（指南已记）；`table/layout.rs` 的 `impl Layout` 分成 57 行和 385 行两段；`table/find.rs` 的 `impl FindController` 分成 90 行和 152 行两段，中间夹着 `impl Highlights`；`table/data.rs:139` 文件中间 `pub use`；`panels/inspector/mod.rs:30-31`、`311-312`，`panels/filter/mod.rs:61-62`、`345-346` 的 `mod` 声明在文件中间；`table/controls/mod.rs:581` 的 `mod status` 在文件末尾。
- **M2 测试位置**（已核实）：`ui/errors.rs:121` 内联测试（指南已记）；`platform/clipboard.rs:112-140` 在内联 `mod macos` 里写 `#[test]`；`gewu-app/src/analysis.rs:60-62` 测试声明在文件中间；Core 有 29 个内联 `mod tests { … }`。
- **M6 可见性**（已核实）：`TableData` 在 `pub(crate) mod data` 里，方法却混用 `pub` 和 `pub(crate)`，字段 `pub columns`（`data.rs:143`）；`TableView` 的 `pub(crate)` 字段 `data`、`document_title`、`worksheet_tabs`、`scroll`、`find`（`mod.rs:49-51`、`55`、`65`）；`FindController.open`（`find.rs:74`）；`TableColumns` 的方法一律 `pub`。
- **几何字面量**（已核实，判断是否要处理）：`table/mod.rs:161` 初始视口 `800.0 - HEADER_HEIGHT`；`input.rs:248` 滚动条显示 650 ms；`shell/info.rs:91` 窗口高度公式 `108. + 25. * …`；`paint.rs` 表头内的 `1.`、`17.`、`13.` 偏移。

---

## 四、按指南确认的正例（保留并推广）

- U1 / U2：`gewu-app/src/query.rs` 的 `QuerySession` 和 `table/query.rs:407-452` 的外壳；`panels/draft.rs` 的 `Draft` 是筛选面板的纯模型，有 4 个纯测试。
- S2：`Drawer` 枚举保证同时只开一个抽屉（`controls/mod.rs:93-114`）；`Load`（`document.rs`）；Inspector 的 `Showing`；`ExportController` 是枚举，`WorkingExport` 被丢弃时取消写入（`export.rs:35-40`）；`ColumnWidth`；`TableColumns` 的 `schema` + `order`。
- S3：prepaint 算布局、paint 后 `cx.defer` 单点提交（`render.rs:142-195`、`layout.rs:350-382`）。
- S4：菜单和快捷键的可用状态由“是否注册处理器”推出（`render.rs:303-359`）。
- S6 / C3：任务句柄存在所有者字段里，替换即取消（Inspector `task`、FilterEditor `load`、`query_task`、`copy_task`）。
- C1：Inspector 和 FilterEditor 用 `Arc::ptr_eq` 判断快照是否变化（`inspector/mod.rs:182`、`filter/mod.rs` `set_snapshot`）。
- F1 / F2：文本按“列 × 64 行块”缓存并有字节上限（`data.rs:42`、`258-274`）；无障碍节点只在激活时构建。
- L5 的 `unsafe` 部分：全部在 `platform/macos/`。
- E4：帧构建里的断言带说明（`frame.rs:12-16`）。

---

## 五、待定条款下观察到的现象（不计违规）

- **T3**：`ViewRow(s.head.row)` 这类就地包装（`controls/mod.rs:130`）依赖公开字段；改私有会逼出 B5 的统一。
- **E3 范围**：按钮、菜单、面板标题全是内联英文（`menus.rs` 菜单项、`render.rs:226` 空结果提示、`document.rs` 的 “Opening…” 等）。
- **V5**：`text_size(px(..))` 字面量仍是 34 处；查找栏、标题栏、信息窗口各有自己的 `px(28.)`、`px(52.)`、`px(13.)`。
- **U4 命令表**：见 B3，证据支持建表。
- **G1**：`set_selection` 不通知，调用方补（如 `render.rs:119-120`）；`copy_finished`、`FindController` 的方法自己通知。两种做法并存。
- **G3**：除指南列出的 `cfg(test)` 访问器外，还有 `cfg(feature = "ui-capture")` 的三个访问器（`table/mod.rs:305-335`）和 `FindController::input_focused`（`find.rs:95-98`）。
- **G4**：`FindViewportChanged` 只有一个订阅者且就是自己（`layout.rs:292-294`、`380`；`mod.rs:198-200`），换成在 `commit_grid_layout` 末尾直接调 `refresh_highlights` 不损失任何东西（判断）。
- **M2 场景测试**：`edge_tests`、`gesture_tests`、`workflow_tests` 在 `table/mod.rs` 里直接 `mod`，不走 `#[path]`。A1 做完后它们大部分会变成模型测试，命名问题会小很多。
- **M7**：实现 `Render` 的类型有 `TableView`、`FindController`、`ExportController`（一个枚举）、`FilterEditor`、`Inspector`、`Document`、`Updater`、`FileDetails`、`WorksheetTabs`、`HeaderTip`、`ColumnDrag`；`Controller` 后缀既用于大外壳也用于状态枚举。
- **M8**：`gewu-gpui` 自写 `version = "0.1.0"`、`edition = "2024"`；`gewu-update` 用 2024；其余继承 workspace 的 2021。
- **M1**：`gewu-core/Cargo.toml` 的 `[lints]` 仍为空。
- **gewu-update**：helper 用 `Box<dyn Error>`（`release.rs:6`）和 `String` 错误，和 App 之间靠 stdout 版本号、stderr 文本通信。对独立 CLI 这是生态常见做法（P6），但 App 一侧因此只能存 `String`（B1）。

---

## 六、对指南本身的反馈

这次审查也是在试用指南，以下几处需要改：

1. **D2 的措辞太窄也太宽。** Rust 里把一个类型的 `impl` 拆到多个文件是正常做法（P6），问题不在“子模块 impl 父类型”，而在“借此让一个 Entity 承担多项职责、所有字段对所有子模块可见”。建议改为：拆文件可以，但每个 `impl` 文件只能动它职责内的字段；判断看 D1 和 U2。现在的历史反例应改成指向 A1 这种形态。
2. **C1 的“身份比较”需要区分对象。** 快照用 `Arc::ptr_eq`；请求键（如 `Import`，`document.rs:185`）按值比较是对的，因为同样的参数产生同样的结果。建议补一句。
3. **L5 要说清“platform::macos”的边界。** `platform/clipboard.rs` 里自建 `mod macos`，按字面算不算？建议统一为“全部 Objective-C 调用都在 `platform/macos/` 目录”。
4. **U4 对鼠标事件的要求要允许“命中测试后分派一个意图”。** `mouse_down` 本来就要先判断点在哪；规则应针对“分派后在闭包里直接改多个字段”。
5. **G2 补一条：表头内部的子区域（漏斗、分隔线）也由 `Layout` 给出。** 这是 B4 的来源。
6. **第六节“现象”的计数要更正：** E3 的 Display 兜底是 7 处，不是 3 处。

---

## 七、建议的改进顺序

按依赖和收益排序（判断）：

1. **A2 文档级状态移到 `Document`。** 先做，它会删掉 `inherit_document_state`、`document_title`、`worksheet_tabs` 和导出的补丁注释，让 `TableView` 的边界变清楚。
2. **A1 抽出 `GridModel`，同时做 B4。** 把交互测试迁成纯测试。
3. **A3 Find 模型化，FilterEditor 改枚举，Document 导入调度模型化，同时做 B6。**
4. **A4 平台隔离和更新功能搬家。** 范围小，可以和 1 并行。
5. **A5 格式知识进 Core。** 需要 Core README 和测试同步（L2）。
6. **B1、B2 错误保留到显示。** 在 1–3 改动对应模块时顺手做，不单独开一轮。
7. **B3** 等 U4 命令表的决定；右键菜单改 action 不依赖这个决定，可以先做。
8. **机械约定（第三节）** 最后一次性扫完，避免和结构改动冲突。

每一步都建议先提交基线，热路径相关的（1、B4）按 `DEVELOPMENT.md` 的 Measure scrolling 和基线做 A/B。

---

## 附：覆盖范围

- 逐行读过：gewu-app 全部；gewu-gpui 的 `shell/` 全部、`table/{mod, data, query, find, render, input, frame, header, layout, selection, columns, find_style}.rs`、`table/controls/{mod, export}.rs`、`table/controls/menus.rs` 后半、`panels/mod.rs`、`panels/inspector/mod.rs` 前 340 行、`panels/filter/mod.rs` 前 200 行、`ui/errors.rs`、`platform/{mod, clipboard}.rs`、`preferences/mod.rs`、`actions.rs`、`lib.rs`。
- 只看了结构、签名或用 grep 查证：`table/{paint, accessibility, scroll}.rs`、`table/controls/{columns, sorting, status}.rs`、`ui/mod.rs`、`ui/messages.rs`、`platform/{update, recent}.rs`、`theme/`、测试文件的组织方式；gewu-core 的模块结构、公开导出、内联测试、模块注释和文件格式部分；gewu-update 的错误类型；`tools/` 只看了目录。
- 没有读：`panels/charts/*`、`panels/inspector/{column, controls, sections}.rs`、`panels/filter/{range, render, choices}.rs`、`panels/{draft, choice}.rs` 的实现细节、`preferences/{window, file}.rs`、`capture.rs`、`examples/` 的实现、各测试的内容。这些部分的问题可能被漏掉，尤其是 `panels/` 内部的状态写法。
- 没有运行任何构建、测试或性能测量。
