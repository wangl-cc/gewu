# gewu 工程指南（草稿）

状态：草稿，2026-11-27。用作 gpui 版本完整 review 的标准；在 review 里证明好用以后，精简并迁进 `docs/dev/guide.md`（那份文档的标题本来就是 Engineering guide）。

依据的代码：`native-gpui` 分支 **33e7677**（2026-11-27 15:08 UTC，远端最新）。下文的 `文件:行` 都指这个提交，路径省略 `crates/` 前缀。标“历史”的例子当时出现在未提交的工作区里，不在 git 历史中，出处是 `gpui-research/` 下的评审文档。

## 怎么用

- 收录标准：这条规则是否在约束**代码怎么写、怎么组织**。产品行为、验收数值、工作流程不放这里，只引用。
- 每条规则有编号（P、L、S、T、E、C、V、U、G、Q、F、D、M），review 结论直接引用编号，例如“违反 T2”。一件事只有一条主规则；具体场景的落法写在主规则下面，或者引用它。
- ✓ 是正例，✗ 是反例，都是代码里真实出现过的片段。
- 标 **待定** 的条目需要 Loong 拍板，拍板前 review 只报现象，不按违规算。
- 和其他文档的分工：谁负责什么写在 `architecture.md`；产品行为写在 `docs/spec/`；色值和几何写在 `docs/dev/design/visual-design.md`；命令、验证方法和通过标准写在 `DEVELOPMENT.md`；agent 的工作规则（回归测试、验证报告、提交）写在 `AGENTS.md`。
- 不写：测试名、易变细节、工具已经强制的东西、评审流程。

---

## 1. 原则

先看可维护、可审阅，而不只是“能跑”。修完一个问题以后要问：实现本身是不是更容易理解、修改和审阅了？“bug 修了、测试过了”回答不了这个问题。

| | 原则 | 检查问题 |
|---|---|---|
| P1 | **一个模型，一种做法。** 同类的事用同一种方式处理；不一致本身就是问题，不当小事。替换一个概念时，类型、名字、注释、测试、文档里的旧概念全部清掉。 | 同类的事，这里和别处做法一样吗？还有没有旧概念的残留？ |
| P2 | **过度工程和工程不足都要查。** 过度工程是抽象没带来收益；工程不足是脚本式平铺、职责混在一起。 | 删掉这层抽象会损失什么？这个函数是不是同时管了好几件事？ |
| P3 | **在源头建立不变量，不到处写防御性检查。** | 这个罕见状态为什么会出现？哪里应该保证它不出现？保证之后，各处的检查还需要吗？ |
| P4 | **状态说得清。** 每份状态都答得出：归谁管、谁能改、什么时候失效、一次操作会连带触发什么。（展开见 2.2） | 读这段代码的人能看清这次调用改了哪些持久状态吗？ |
| P5 | **新增的复杂度要讲得出理由。** 行数是追问设计成本的信号。 | 多出来的代码换来了什么？消除了哪些重复、耦合或理解成本？ |
| P6 | **符合生态惯例。** Rust 和 gpui 生态里通常的做法是默认选项，偏离要有理由。 | 生态里这件事一般怎么做？我们不一样的理由是什么？ |
| P7 | **移植时判断什么值得保留。** 不必逐行对照旧实现。 | 能说出这个行为为什么必须保留吗？这个历史做法还有存在的理由吗？ |

---

## 2. 设计规则

### 2.1 分层

依赖方向：`gewu-gpui → gewu-app → gewu-core`，UI 也可以直接用 Core 类型。各层具体负责什么见 `architecture.md`。

- **L1 Core 只放通用数据语义。** 不出现窗口、显示单位、文案、交互状态这类前端概念。检查：这个类型或参数换一个前端还有意义吗？
  - ✓ Core 的视图不保存列投影、查询条件和窗口状态，可见列顺序归调用方（`gewu-core/README.md`，Dataset 一节）。
- **L2 Core 的公开 API 当公开 API 来设计。** UI 需要新的细粒度能力时，先按通用数据语义设计命名和职责，不为某个界面顺手加函数；每个新接口同步写进 Core README 并有测试。检查：去掉 gewu 的界面，这个接口还说得通吗？
  - ✓ `OrderedView::exact_filter_for_cell`（按单元格值构造筛选）以通用语义命名，记在 `gewu-core/README.md:19`。
- **L3 gewu-app 放应用语义，不依赖 gpui。** 快照复用、查询合并、分析缓存、Find 分页。
  - ✓ `gewu-app/Cargo.toml` 只依赖 `gewu-core`；`gewu-app/src/lib.rs:2` `#![forbid(unsafe_code)]`。
- **L4 能脱离窗口表达的逻辑往下放。** 判断标准：能不开窗口就写出单测的规则（调度、选区、滚动、布局几何及其编排），放进 gewu-app 或 UI crate 里不依赖 gpui 的模块；gpui 代码只留“在哪个线程跑、什么时候 spawn、怎么画”。具体写法见 U1。
- **L5 平台代码隔离。** `unsafe` 和 Objective-C 调用只出现在 `platform::macos` 和 `gewu-update`；gpui-fast 和 gpui-pre 的差异只出现在 `platform` 模块里。
  - ✓ `gewu-gpui/src/platform/mod.rs:28-30` 是全 crate 唯一的 `feature = "gpui-fast"` 分支。

### 2.2 状态、生命周期、副作用

- **S1 每份状态只有一个所有者。** 别的模块要改它，只能通过所有者的方法或事件。在 gpui 里，所有者就是 Entity：别的 Entity 通过方法、`update` 或事件（`EventEmitter` + `subscribe`）和它交互，不直接读写它的字段。字段含义不明显时，用一行注释写清它是什么、什么时候失效。
  - ✓ `gewu-gpui/src/table/columns.rs:29-30`：`VisibleCol` 的注释写明“有效期到列被隐藏或重排为止”。
  - ✓ `gewu-gpui/src/table/find.rs:150` `impl EventEmitter<CellMatch> for FindController`，`TableView` 在 `table/mod.rs:171` 订阅。
- **S2 不允许构造出业务上无效的组合。** 几个必须一起变的字段合成一个 enum 或一个结构体，不用几个平行字段或平行集合手动同步。
  - ✓ `gewu-app/src/query.rs:8-12` `enum Phase { Idle, Running(..), Failed(Error) }`。
  - ✓ `gewu-gpui/src/table/columns.rs:7-12` `enum ColumnWidth { Unmeasured, Measured(f32), Manual(f32) }`，加上 `TableColumns` 的 `schema` + `order`（39-43 行）。
  - ✗ 历史：`query_started`、`query_error`、`query_task` 三个字段拼出查询状态；`columns`、`widths`、`names`、`width_sources` 四个平行集合手动同步（`native-gpui-review-and-visual-spec.md` 第 4、6 条）。
- **S3 渲染路径不改持久状态。** render、prepaint、paint 只读状态，计算结果当返回值往下传；不调 `notify`、不 spawn 任务、不创建会话（绘制阶段调 `notify` 也不会触发下一帧，见 gpui-fast `window.rs:171-196` 的 `invalidate_view`）。需要保存的部分，帧结束后用 `cx.defer` 从**一个**明确的入口提交。缓存可以在渲染路径里填充，前提是它对语义不可见、丢掉以后能重建。
  - ✓ `gewu-gpui/src/table/render.rs:142-195`：prepaint 算出 `GridFrameLayout`，paint 把它放进 `pending_layout`，再用 `cx.defer` 调 `commit_grid_layout`（`table/layout.rs:350-382`）统一提交列宽、视口和水平滚动。
  - ✗ 历史：canvas 的 prepaint 里用 `entity.update` 直接写 `origin`、`scroll`、`scroll_x`、`layout`，同一闭包里测文字并改列宽（同上评审文档第 2 条）。
- **S4 能算出来的就不存；必须存两份时说清角色。** 可用状态、计数、摘要这类能推出来的值，在需要的地方现算。真要缓存或保留副本（比如“请求值”和“已提交帧”），在名字或注释里写清各自的角色、同步点和谁覆盖谁。
  - ✓ `gewu-gpui/src/table/render.rs:355` `.when(self.can_export(cx), |view| view.on_action(…))`：菜单项是否可用由 gpui 根据焦点路径上有没有这个处理器推出来（gpui-fast `platform/app_menu.rs:341`），不另存 enabled 标志。
  - ✓ `gewu-gpui/src/table/mod.rs:72-73`：`layout` 的注释写明它是“绘制和命中测试共用的已提交几何”，和请求值 `scroll`、`horizontal` 区分开。
- **S5 一个操作的连带效果写在一处。** 结果生效时要连带清理或刷新的状态，在同一个函数里按顺序列出来，不分散到各处的回调里。
  - ✓ `gewu-gpui/src/table/query.rs:97-103`：查询结果装上以后，在一个分支里依次更新行数、清空选区、处理待隐藏列、重置表格交互、刷新抽屉。
- **S6 任务和订阅的生命周期绑在所有者上。** 后台任务句柄存在所有者的字段里，替换或丢弃句柄就取消任务；被取消的任务结束时不改任何状态。订阅和长期任务存成 `_name: Subscription` / `_task: Task<()>` 字段，随 Entity 销毁结束，不手动管理。
  - ✓ `gewu-gpui/src/table/mod.rs` 的 `query_task`、`copy_task`（70-71 行），`_style_changes`、`_find_reveal` 等字段。

### 2.3 类型

- **T1 含义不同的值用不同的 newtype。** 源数据行号、视图行号、列标识、可见列位置不能都是裸整数。
  - ✓ `gewu-core/src/view/column_segment.rs:56-62` `SourceRow`、`ViewRow`；`gewu-core/src/column/id.rs:7` `ColumnId`；`gewu-gpui/src/table/columns.rs:31` `VisibleCol`。
- **T2 newtype 要一直用到使用处。** 不要在边界上拆回裸整数，到用的地方再包回去；拆包和换算只在一处。元组字段超过两个、或者含义不能从类型看出来时，改成具名结构体。
  - ✗ `gewu-gpui/src/table/selection.rs:5-9` `Cell { row: u64, column: usize }`。
  - ✗ `gewu-gpui/src/table/data.rs:166-176` `display_cell` 收到 `ViewRow` 和 `ColumnId`，返回的却是裸 `u64`、`usize`。
  - ✗ `gewu-gpui/src/table/controls/mod.rs:130` `ViewRow(s.head.row)`，在用的地方重新包装。
  - ✗ `gewu-gpui/src/table/layout.rs:50-51` `columns: Vec<(usize, f32, f32)>`，三个字段的含义靠注释说明。
- **T3 构造函数放在含义成立的地方。** 不是任何原始值都合法的类型，字段设为私有，只开放有意义的构造方式。
  - ✓ `ColumnId(u32)` 字段私有（`gewu-core/src/column/id.rs:7`）。
  - **待定**：`SourceRow(pub u64)`、`ViewRow(pub u64)` 是公开字段，要不要改成私有。
- **T4 一个概念只用一个名字，每一层都一样。** 必须翻译到工具包的命名时，翻译只放在一个函数里。
  - ✗ 类型叫 `ViewRow`，字段和方法却叫 `display_row`（`CellMatch.display_row`、`display_row_for_source`、`display_cell`）。
  - ✓ gewu 的 `accent` 和 gpui-kit `ColorTokens.accent` 含义不同，映射只在 `gewu-gpui/src/theme/mod.rs:167` `base_colors` 一处。

### 2.4 错误

- **E1 错误是结构化的类型，按拥有失败操作的模块划分，保留 source。** 只有违反契约才算 `Internal`。
  - ✓ `gewu-core/src/error.rs` 的 `Error` 按 File / Column / Filter / Find / Analysis / Export 分类，`From` 实现把嵌套的 Internal 提到顶层。
- **E2 错误值一直保留到显示的那一刻。** 状态里存 `Error`（或带描述所需值的结构化 Kind），渲染时才转成文字。i18n 需要这一点。
  - ✓ `gewu-app/src/query.rs:11` `Failed(Error)`；`gewu-gpui/src/shell/document.rs:23-27` `Load::Failed(Import, gewu_core::Error)`。
  - ✗ `gewu-gpui/src/table/query.rs:5-9` `FilterFailed(String)`、`CopyFailed(String)`；`table/find.rs:63` `Error(String)`；`table/controls/export.rs:26` `Failed(String)`。
- **E3 用户可见的文案集中在一处，并且穷尽匹配。** 从错误到文案的映射不用通配分支，也不把 Core 的 `Display` 文本直接给用户看，这样新增一种错误时编译器会指出每一处要补的地方。
  - ✓ `gewu-gpui/src/ui/errors.rs:1`：文案在 UI 边界上选，这时错误还保留着类型。
  - ✗ 同一文件 76、103、113 行：`Error::File(e) => e.to_string()` 这类兜底分支。
  - **待定**：文案的边界是“控件标签归控件，句子归 `messages.rs` / `errors.rs`”，还是全部集中（`review-547899b.md` §7 第 5 条）。
- **E4 可以恢复的情况不 panic。** 产品代码不用 `unwrap` / `expect`。断言只用来表达不变量，要带说明。
  - ✓ `gewu-gpui/src/table/frame.rs:12-16` `assert_eq!(…, "the viewport must fit the displayed view")`。

### 2.5 并发与异步

调度规则本身（同时只跑一个、只保留最新、失败后回到哪里）按 U1 写成纯模型。这里写异步结果的约定。

- **C1 后台结果是不可变快照，只发布仍然有效的结果。** 失败也一样：过时的失败直接丢弃；当前请求失败时保留上一次成功的显示。判断“是不是同一个”用身份比较（`Arc::ptr_eq`），不用内容比较去猜。
  - ✓ `gewu-gpui/src/panels/inspector/mod.rs:182` `Arc::ptr_eq(self.snapshot.filtered(), snapshot.filtered())`。
- **C2 任务开始时就把输入定下来。** 复制和导出在发起时捕获当前显示的快照和列顺序，不在任务里读可变状态。
  - ✓ `gewu-gpui/src/table/mod.rs:368-377` `copy` 在 spawn 之前克隆 view、列 ID 和行范围。
- **C3 取消靠丢弃句柄或协作式标志。** Core 的内存计算不支持取消，这是有意的设计。

### 2.6 视觉

具体色值、字号、几何以 `docs/dev/design/visual-design.md` 为准，这里只写代码层面的规则。

- **V1 颜色只来自主题 token。** `theme/` 以外不出现颜色字面量。
  - ✓ 33e7677 上用 grep 找 `rgb(`、`rgba(`、`hsla(`、六位十六进制，`theme/` 以外一处都没有。
- **V2 强调色由主题固定，不跟随系统。** 前景色按角色和填充色成对定义（`accent` / `accent_foreground`），由主题写死，代码不在运行时计算明暗或对比度。
  - ✓ `gewu-gpui/src/theme/palettes/default.rs`：`accent: rgb(0x007aff)`、`accent_foreground: white()`。
- **V3 默认主题不以对比度数值为目标；高对比度是同一套 token 的另一组取值。** 组件里不为高对比度写分支。颜色以外的提示（描边粗细等）单独放一组参数。
  - ✓ `theme/palettes/high_contrast.rs`；`theme/mod.rs` 的 `VisualCues` 在“区分时不只用颜色”打开时加粗描边。
- **V4 主题测试只测结构不变量，不测对比度比值。**
  - ✓ `gewu-gpui/src/theme/tests.rs:146-153`：暗色下 elevated 比 surface 亮、surface 比 canvas 亮、分段滑块比轨道亮。
- **V5 待定：字号、间距、圆角要不要也走 token。** 视觉规范定了三级字号，但组件里有 34 处 `text_size(px(..))` 字面量。

### 2.7 UI 写法：模型 / 外壳 / render

**主规则。** 每个功能拆成三块，数据单向流动，副作用显式。gpui 的所有权按 Entity 划分，notify 也按 Entity 生效；gpui-fast 靠“没被 notify 的视图复用上一帧”来提速。所以**不用**单一全局 store（Redux 式），而是让每个 Entity 管自己那一块，把判断逻辑抽到它持有的纯模型里。

- **U1 模型：纯 Rust，不依赖 gpui。** 它持有一个功能的状态，公开方法的形状是“意图 → 新状态 + 要执行的副作用”。副作用用返回值表达（要跑的任务、要发的通知），模型自己不 spawn、不 notify，不需要窗口就能单测。应用语义的模型放 gewu-app，交互状态的模型放 UI crate 里不依赖 gpui 的模块（L3、L4）。“哪个结果有效”这类调度规则就是模型的一部分。
  - ✓ `gewu-app/src/query.rs:83` `QuerySession::edit` 返回 `Option<QueryJob>`，`:106` `finish` 返回 `QueryCompletion`，其中 `Run(QueryJob)`（`:42`）表示“接着跑这个任务”；调度规则由 `query_tests.rs` 不开窗口测。
  - ✓ `gewu-gpui/src/table/scroll.rs`、`selection.rs`：滚动和选区的计算部分。
- **U2 外壳：Entity 只做三件事。** 把输入翻译成对模型方法的调用；执行模型返回的副作用（spawn 任务，把结果交还给模型）；`cx.notify()`。“哪个结果有效”“结果生效后连带改什么”这类判断不写在外壳里。外壳持有模型、任务句柄、订阅和焦点。
  - ✓ `gewu-gpui/src/table/query.rs:72-91` `edit_query`：只负责跑 job、交回结果、按返回值决定下一步。
- **U3 render：只从模型和外壳状态生成元素树**（S3）。无状态的部件写成返回元素的函数或 `RenderOnce` 组件；只有需要自己的生命周期、焦点或异步任务时，才单独做成 Entity。
  - ✓ `gewu-gpui/src/ui/mod.rs:122` `button`，以及 `segment`、`segmented_tabs` 等函数。
- **U4 用户意图是数据。** 键盘和菜单命令只走 action + keybinding，不在 `key_down` 里硬匹配快捷键；应用级命令全局绑定、处理器挂在 App 上（`shell/mod.rs`），文档级和表格级命令带 key context 绑定（`actions.rs`）；可用状态按 S4 推出来（状态允许时才注册处理器）。鼠标事件的处理器也只把事件翻译成一次意图（一个 action 或一次模型方法调用），不在闭包里直接改好几个字段。
  - ✗ `gewu-gpui/src/table/render.rs:126-136`：`on_hover` 闭包里直接清 `header_hover`、`thumb_hover`、`body_pointer`，和 `table/mod.rs:233` `reset_grid_interaction` 清的那组字段重叠又不完全一样。
  - **待定**：要不要建一张命令表（每个命令一行：action、标题、快捷键、context、菜单分组），让快捷键绑定和菜单都从它生成。现在一个命令的快捷键在 `actions.rs` 或 `shell/mod.rs` 的 `bind_keys` 里，标题和菜单位置在 `shell/mod.rs` 的 `update_menus` 里。

**gpui 细则**（只写现在能确定的）

- **G1 改了 render 会读的状态，就必须 `cx.notify()`。** gpui-fast 的视图是 retained subtree，依赖没变就复用上一帧，不通知就不会重画（gpui-fast `src/fast/retained.rs` 模块注释）。
  - **建议，待确认**：由改状态的外壳方法自己调 `notify`，调用方不再补。现在两种都有：`table/controls/mod.rs:117` `set_selection` 不通知，由调用方补；`table/mod.rs:358` `copy_finished` 自己通知。
- **G2 自绘表格的状态归属。** 持久的交互状态（选区、滚动锚点、水平滚动、拖动、悬停）及其编排按 U1 归一个不依赖 gpui 的表格交互模型，`TableView` 只当外壳；列宽在 `TableColumns` 里。`Frame` 是每帧算出来的不可变值，整个移进 canvas，用完就丢（`table/mod.rs:116-117`）。已提交的 `Layout` 是绘制、命中测试、无障碍和弹层锚点共用的唯一几何；处理输入前先提交还没提交的帧（`table/layout.rs:351-352`）。
- **G3 待定：产品类型上要不要允许 `#[cfg(test)]` 的访问器和变体。** 例如 `table/mod.rs:277-297` 的 `gutter_width`、`column_width`、`thumb_bounds`，`platform/clipboard.rs:43` 的 `TestContents`。按 U1 抽出模型以后，这类访问器大多会变成模型上的普通方法。
- **G4 待定：实体给自己发事件再自己订阅，算不算合理的间接。** 例如 `table/layout.rs:380` emit `FindViewportChanged`，`table/mod.rs:198` `subscribe_self`。还是应该直接调用？

### 2.8 测试

文件放在哪里见 M2；回归测试必须在没有修复时失败、报告时分开说清验证了什么，见 `AGENTS.md`。

- **Q1 行为在拥有它的那一层测一次。** 数据语义在 Core 测，查询合并、缓存、Find 分页在 gewu-app 测，交互判断在 UI 的纯模型上测（U1）。同一个行为不在几层重复测。窗口测试只测接线：输入有没有变成正确的意图、副作用有没有被执行、结果有没有画出来。如果一条逻辑只能开窗口测，先问它是不是该下沉到模型里。
  - ✓ `gewu-app/src/query_tests.rs`：“过时的失败只会触发最新的编辑”这类调度规则不开窗口就能测。
- **Q2 测试按行为命名和组织。** 测试名是一句描述行为的话，不叫 `test_foo_1`；一个测试只验证一个行为。

### 2.9 热路径

render、prepaint、paint 以及它们每帧调用的函数是热路径，滚动时每帧都要跑。性能要求只针对默认的 gpui-fast；gpui-pre 只保证能编译、能用。

- **F1 热路径上的开销和可见区域成正比，而不是和数据量成正比。** 不格式化或拷贝整列；不按单元格分配不必要的内存；不做阻塞 I/O；昂贵的、可复用的结果按可见块缓存。
  - ✓ `gewu-gpui/src/table/data.rs:3-5`：按“列 × 64 行块”向 Core 取格式化文本并缓存；`:18-22` 单元格预览最多取 1,024 字节。
  - ✓ `gewu-gpui/src/table/render.rs:150-152`：只有无障碍处于激活状态时才构建无障碍节点。
- **F2 有意的例外：可见区域在主线程同步读。** 这样跳转时不出占位符，代价是可见区域的读取必须足够快。其他重计算（排序、筛选、统计、导出、Find 计数）一律放后台。
- **F3 改动热路径后按 `DEVELOPMENT.md` 的 Measure scrolling 实测，并和基线比较。** 测量方法和通过标准写在那里（见第 4 节）。

### 2.10 模块拆分

- **D1 按职责拆，不按行数拆。** 一个模块对应一个能用一句话说清的职责，这句话写进它的 `//!`（M4）。行数只是信号：文件超过约 400 行时，问一句它是不是同时管了几件事，但不为凑行数硬拆。
  - ✓ `gewu-gpui/src/table/`：按接缝拆成 `layout`（几何）、`frame`（每帧数据）、`paint`（绘制）、`render`（元素树）、`input`（输入）、`query`（查询调度）、`find`（查找）。
- **D2 模块之间不循环依赖。** 子模块通过父模块定义的类型协作，不互相 `impl` 对方的类型。
  - ✗ 历史：`find.rs` 写的是 `impl TableView`，和 `table.rs` 互相依赖（`native-gpui-migration.md` 附录 A）；现在 `FindController` 是独立 Entity，通过事件通知表格。

---

## 3. 机械约定

工具已经强制的（rustfmt 配置、workspace clippy lints，包括用 `self_named_module_files` 强制 `mod.rs`、CI 里 `-D warnings`、cargo deny）不在这里重复，只写工具管不到的部分和例外。

- **M1 lint 例外。** 用 `#[expect(lint, reason = "…")]`，范围尽量小；`unsafe` 块另外写 `// SAFETY:`。只有 lint 是否触发取决于 cfg 或 feature 时才用 `allow`，并在旁边写原因。
  - ✓ `gewu-gpui/src/platform/macos/mod.rs:11-15`。
  - ✗ `gewu-core/src/file/mod.rs:230` `#[allow(unreachable_patterns)]`：用 `allow` 是对的（是否触发取决于 feature），但没写原因。
  - **待定**：`gewu-core/Cargo.toml` 的空 `[lints]` 让 Core 不吃 workspace lints。保留，还是接入、只在测试和 bench 里放开 `unwrap` / `expect`？
- **M2 单元测试放同级文件。** `foo.rs` 配 `foo_tests.rs`，`mod.rs` 配 `tests.rs`，在源文件**末尾**声明 `#[cfg(test)] #[path = "foo_tests.rs"] mod tests;`。跨模块共享的夹具放 `foo_fixture.rs`，声明为 `mod fixture`。测试公开 API 的跨模块测试放 `crates/*/tests/`。不写内联的 `mod tests { … }`。
  - ✓ `gewu-app/src/query.rs:132-134`。
  - ✗ `gewu-gpui/src/ui/errors.rs:121` 内联测试；`gewu-app/src/analysis.rs:60-62` 的测试声明夹在文件中间。
  - **待定**：按场景组织、需要访问私有项的 UI 测试（`table/gesture_tests.rs`、`edge_tests.rs`、`workflow_tests.rs`、`ui_fixture.rs`）怎么命名、怎么声明。
- **M3 文件内顺序。** `//!` 模块注释、crate 属性、`mod` 声明、`use`（一组）、常量和类型、`impl`、私有辅助函数，最后是测试模块声明。`use` 不夹在常量或 `mod` 中间。
  - ✗ `gewu-gpui/src/table/mod.rs:30` `use layout::HorizontalScroll;` 在常量后面；402-430 行 `mod` 和 `use` 交替出现。
- **M4 `//!` 模块注释写什么。** 每个非测试模块文件都要有：这个模块负责什么、它维持什么不变量、关键常数及其理由（比如块大小、分页上限）；模块多的 crate 在 `lib.rs` 写一份 code map。不写历史（“不再有 X”），不复述代码。README 只写对外的入口和 API 契约，算法细节和常数放 `//!`。
  - ✓ `gewu-gpui/src/table/scroll.rs:1-4` 讲清为什么用 `u64` + `f64`；`table/data.rs:3-5` 写明按“列 × 64 行块”缓存。
  - ✗ `gewu-gpui/src/table/data.rs:1` “no FFI, no packing” 讲的是历史；`gewu-core/src/lib.rs:3` “The Mac app …” 是旧概念的残留。
- **M5 注释讲为什么，不讲做什么。** 公开项的契约不能从类型看出来时，写 doc comment。
  - ✓ `gewu-gpui/src/table/render.rs:168-169` “The grid owns a drag until mouse-up …”。
- **M6 可见性尽量小。** 实体的字段默认私有，不为测试或方便把字段开成 `pub(crate)`。
- **M7 命名。** 名字说明职责，不用 `utils`、`helpers`、`manager` 这类空泛的名字；一个概念一个名字见 T4。
  - **待定**：实现 `Render` 的类型怎么命名。现在后缀很杂：`TableView`、`FindController`、`ExportController`、`FilterEditor`、`Inspector`、`Settings`、`Document`。是约定后缀来表达 U2 的“外壳”角色，还是明确不靠后缀表达角色？
- **M8 Cargo 元数据统一。** `version`、`edition`、`license` 和共享依赖从 workspace 继承。
  - **待定**：`gewu-gpui` 和 `gewu-update` 用 edition 2024，其他 crate 用 2021；`gewu-gpui` 没有继承 `version`。统一成哪个？

---

## 4. 迁入仓库时的处理

不是规则，是把草稿迁成 `docs/dev/guide.md` 时要一起做的事。

- 去掉所有 `文件:行`，例子只留模式描述，避免过时。
- 性能要求拆成两半：用户看得到的行为（跳转后立刻显示完整的数据和行号、不出现占位符或空白；滚动跟手）写进 `docs/spec/interface.md`，它现在没有这一条；数值通过标准和测量方法写进 `DEVELOPMENT.md` 的 Measure scrolling，标准为：gpui-fast、release 构建、真实显示器，慢滚、快滚、跳转的 render→paint p95 都 ≤ 8 ms（够 120 Hz，偶尔掉一帧可以接受），零空白；改动热路径时和基线做 A/B 对比，不能明显退化。迁移计划里的旧门槛（慢滚 ≤ 2 ms、跳转 p50 ≤ 4 ms）作废。
- `docs/dev/native-acceptance.md` 是迁移阶段的验收记录，迁移已通过验收：挪进 `docs/dev/design/`，和迁移计划放在一起，标为已完成并冻结。仍然有用的少数内容提出来放到对应位置，比如哪些行为必须在真窗口或 VoiceOver 下手动检查、签名和更新的检查（归 `app-updates.md`）。
- 工作习惯放进 `AGENTS.md`（人用的入口是 `DEVELOPMENT.md`）：大改之前先提交一个可比较的基线；提交要签名；spec 和文档的改动跟着对应代码一起提交。Conventional Commits 已经在 `AGENTS.md` 里。
- 现有 `guide.md` 里和本指南重复的句子（action/keybinding、渲染无副作用、测试同级文件等）以本指南为准合并，不留两份。

---

## 5. 待定汇总

1. T3：`SourceRow` / `ViewRow` 的字段要不要改成私有。
2. E3：文案集中到什么程度（控件标签是否也集中）。
3. V5：字号、间距、圆角要不要走 token。
4. U4：要不要建命令表，让快捷键和菜单都从它生成。
5. G1：`notify` 由改状态的方法负责，还是由调用方负责（建议前者）。
6. G3：产品类型上是否允许 `#[cfg(test)]` 的访问器和变体。
7. G4：实体给自己发事件再订阅，是否接受。
8. M1：Core 是否接入 workspace lints。
9. M2：场景式 UI 测试文件的命名和声明方式。
10. M7：实现 `Render` 的类型靠不靠后缀表达角色。
11. M8：统一 edition 和 workspace 继承。

---

## 6. 写作中注意到的现象

只列现象，不下结论，留给 review。

- 选区、布局、列访问仍用裸下标（`Cell`、`Layout.columns`、`TableColumns::entry/width/set_width(usize)`、`cycle_sort(column: usize)`）（T2）。
- 4 处 UI 状态把错误先转成 `String`；`TableData::prefetch` 返回 `Option<SharedString>` 形式的错误；`shell::report_window_error` 收字符串（E2）。
- `ui/errors.rs` 有 3 个 `to_string()` 兜底分支（E3）。
- Find、筛选面板、Inspector、剪贴板（`CopyGeneration`）各自在 UI 里判断“哪个结果有效”，方式各不相同（U1、C1）。
- `TableView` 约 26 个字段，交互编排只能开窗口测（U1、G2、Q1）。
- 34 个非测试模块文件没有 `//!`，集中在 `panels/`、`table/controls/` 和 `table/` 的 render、paint、layout、frame（M4）。
- Core 还有约 30 个内联 `mod tests { … }`（M2）。
- `gewu-core/README.md` 有大量实现细节和常数；链接的 `DEVELOPMENT.md#rust-core` 锚点不存在（M4）。
- `gewu-app`、`gewu-gpui` 的 `lib.rs` 里 `cfg_attr(test, allow(unwrap/expect))` 可能和 `clippy.toml` 的 `allow-*-in-tests` 重复（推断，未验证）（M1）。
- `GewuPalette` 的 `accent_pressed`、`input_hover` 既是公开字段又有只返回字段的同名 getter（P1）。
- `pub(crate)` 的实体字段：`TableView.data`、`scroll`、`find`，`FindController.open`（S1、M6）。
- 超过 400 行的 UI 文件：`table/find.rs`、`table/controls/mod.rs`、`ui/mod.rs`、`shell/document.rs`、`table/controls/columns.rs`（D1，只是信号）。
- 主题里已有 Catppuccin 调色板，之前的评审建议这一轮先不做（`review-547899b.md` §4）。
- 现有实测在 8 ms 标准内（M4、60 Hz，最高 6.77 ms），物理 4K 和 120 Hz 真机未测；前台 render→paint（约 6 ms）和锁屏强制绘制（约 1.2 ms）差约 5 倍，原因未查明（F3）。
