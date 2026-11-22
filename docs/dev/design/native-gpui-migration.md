# 全 Rust 迁移计划（gpui-kit）

2026-11-22 已在 `native-gpui` 实现候选版本，正在进行发布前验收。计划起草时的基线是 b2056bb（main 9cc7140 加上 GPUI 表格原型）。[验收记录](../native-acceptance.md)

这是 docs/dev/design/ 下的设计文档：按 design/README 的规则，它是历史记录，写完后不随代码更新；现行规则写进 architecture.md 和 guide.md。docs/dev/ 和 docs/spec/ 来自尚未合并的 PR #18（及叠在上面的 #19），本分支合入 main 时会一并带来；在那之前以 PR #19 分支（refs/pull/19/head）上的版本为准。

## 目标与范围

把前端从 Swift（SwiftUI + AppKit）经 UniFFI 调 Rust，改为纯 Rust：gpui-kit + 自绘单 Canvas 表格，UI 直接调用 Core，彻底去掉 FFI。决定性的收益是没有 FFI：Core 的接口不再为跨语言变形。跨平台只是潜在加分，macOS 原生感不是目标。

- 分支策略：本分支直接删除 Swift 前端、GewuKit 和 gewu-uniffi。main 保持可发布的 Swift 版，旧实现从 main 和 git 历史里查。
- 迁移期间 main 只改 Core 和修 bug，不再给 Swift app 加功能；Core 的改动经常合进本分支（merge，不 rebase）。
- 不在范围内：新功能（v1 范围不变：查看、排序、Excel 式筛选、列统计、schema）、Linux/Windows 发布（只保证不阻断）、撤销。
- 对齐基准：docs/spec/ 描述的产品行为，加上从 Swift 测试里提取的行为约定。

## 基线原型

gpui/ 下的原型（基线 b2056bb）已经具备：

- 自绘单 Canvas 表格，直接调用 gewu-core；固定行高，纵横两向只构建可见行列；表头固定，左侧行号列显示源行号。
- 滚动用 u64 顶行号 + f64 行内偏移的锚点，自绘可拖动的垂直滚动条。
- 按 (列, 64 行块) 在 render 时同步调 `format_column` 取字，跳转当帧出字。
- 选区：单击、Shift+点击、拖动、方向键、Shift/Cmd+方向键、Page Up/Down、Home/End、Cmd+A，选区移出视口时自动跟随。
- Cmd+C 走 Core 的 TSV 导出（64 MB 上限）；拖动表头分隔线调列宽，双击自适应。
- 点表头排序、按单元格值筛选（Cmd+Shift+L，Cmd+Option+L 清除）在后台执行，只保留最新请求；查找（Cmd+F，Return / Cmd+G 下一个）。
- 为此给 gewu-core 加了 `OrderedView::exact_filter_for_cell` 和 `ColumnHighlight::matches`，尚未作为 Core API 评审。

## 分层与约束

三层单向依赖：Core → gewu-app → gpui 应用。只有最外一层依赖 gpui。

- gewu-core：只有通用数据语义，没有前端概念。新的细粒度 API（例如按单元格值筛选）当作公开 API 设计和评审，不随功能顺手加。
- gewu-app（新，不依赖 gpui）：app 级语义，从 gewu-uniffi 搬过来：会话、查询（filters + sorts → 不可变视图，筛选不变时复用）、分析缓存、Find 分页、结构化错误（保留错误 Kind，用于本地化）。全部可写纯单测。
- gpui 应用：窗口、表格、弹窗、检查器。交互状态（选区、列宽、筛选草稿）只在这一层。滚动锚点、选区、布局计算写成不依赖 gpui 的纯逻辑。

约束：

- 默认开 gpui-fast；`--no-default-features` 退回 gpui-pre 必须始终能编译。fast 与 pre 的差异只出现在一个 platform 模块里。
- 重计算（排序、筛选、统计、导出）在后台执行，结果以不可变快照交回主线程，过时结果丢弃。可见区域在主线程同步读，跳转不出占位符。
- render 无副作用；命中测试和绘制用同一份布局快照。
- 快捷键只用 action + keybinding 一种方式。
- 一个 workspace、一个 Cargo.lock，cargo deny 覆盖全部依赖。

## 迁移后继续适用的开发规则

以下来自 PR #18/#19 的 docs/dev/guide.md，是本分支所有改动都要遵守的规则。FFI、GewuKit、App 三层被删掉后，它们的规则不会消失，而是落到 gewu-app 或 gpui 应用上。

通用：

- 一个清晰的模型：每件事只留一种做法。替换一个概念时，在类型、名字、注释、测试和文档里全部去掉旧的。同一个名字在每一层含义相同。
- 文档和代码在同一个改动里一起变：层的职责或数据面变了改 architecture，规则变了改 guide，行为变了改 spec。
- 筛选、Find、值语义的改动先改 spec（它是契约），再改 Core 和测试。
- 行为只在拥有它的边界测一次；回归测试在没有修复时必须失败。
- 报告时分清验证过什么：构建、哪些测试、运行中的 app、测量是不同的声明。
- issue 只用于讨论；结论写回 guide、architecture 或设计文档，文档不引用讨论串作为规则来源。

Rust 约定：

- 多文件模块用 `<module>/mod.rs`。
- 单元测试放在同级文件：`mod.rs` 配 `tests.rs`，`foo.rs` 配 `foo_tests.rs`，用 `#[cfg(test)] #[path = "foo_tests.rs"] mod tests;` 声明；没有行数阈值。跨模块共享的测试夹具用 `foo_fixture.rs`，声明为 `mod fixture`。跨模块测试放 `crates/*/tests/`。新写的 gewu-app 和 gpui 应用代码同样遵守。
- 为显示选的单位不进 Core（例如散点图时间坐标换成秒），迁移后放在 gewu-app 或 UI 层。

Core（不变）：

- 筛选总是对整个数据集执行，不基于上一个视图。
- 一列的筛选编辑器忽略该列自己的筛选条件。
- null 在两个方向都排最后；相等的键保持源顺序。
- 精确值和范围边界以文本无损往返；字典编码不离开 Core。
- 查询读原生 Arrow 数组，不格式化或拓宽整列。
- 导出只在完成时替换目标文件，永远不覆盖源文件。
- 错误属于拥有失败操作的模块，并保留 source。
- 每个编解码 feature 单独能构建并通过测试。

原 FFI 层的规则 → gewu-app：

- `apply` 在什么都没变时返回同一个视图，只有排序变了时保留筛选后的视图。UI 跟随这个结果判断复用，不自己决定。
- 分析缓存每个分析、每列只保留一个结果，不缓存失败，大小受 schema 约束。
- 只有违反契约才变成 `Internal`；筛选失败在任何地方都保持 `Filter` 类别。错误保持结构化 Kind，供本地化使用，不折叠成消息字符串。
- 指针借出、`gewu_data.h`、打包值这些数据面规则随 FFI 一起删除。

原 GewuKit 层的规则 → gewu-app：

- 三种结果类型（数据集、筛选视图、排序视图）不可变，按身份比较。
- 筛选字面量按 UTF-8 字节比较，筛选按列顺序保存，所以相等的查询选中相同的行。
- 重计算在后台执行；一个等待者取消不会取消别人共享的分析。
- 编辑规则（表格状态、筛选草稿）在专门的状态类型里，不在视图代码里。

原 App 层的规则 → gpui 应用：

- 只有查询成功时，显示的结果才改变；查询失败时保留原结果，草稿回到它。
- 被取消的任务结束时不改变任何状态，不论成功或失败。
- 行变化时表格永远不显示空白、部分或混合的帧。
- 列的显隐和顺序立即生效，从不触发查询。
- 面板按结果身份、列和参数加载，参数变了旧结果不会装上去。
- 复制就是所选行和可见列的 TSV 导出。

常见改动（迁移后的版本）：

- 新分析：Core 里一个与行顺序无关的 kernel 加能力标志；gewu-app 的筛选视图上一个走分析缓存的方法；一个 Inspector 面板。
- 面向用户的失败：所属 Core 模块错误里一个变体；gewu-app 里带描述所需值的结构化 Kind；UI 里的文案。穷尽匹配会指向每一步。
- 新文件格式：Core `file/`，放在自己的 Cargo feature 后面；禁用的格式仍能编译并报告自己被禁用。更新 spec/file-formats.md 和 README 的格式表。

## 代码质量标准

分两类：机器能卡住的放进 CI 当硬门槛；靠判断的写成评审清单和必测边界，每一步验收时逐条过。迁移完成后，这一节并入 guide.md。

CI 硬门槛：

1. `cargo fmt --check`（仓库用 nightly rustfmt）；`cargo clippy --workspace --all-targets -- -D warnings`，fast 和 `--no-default-features` 两套 feature 都跑。在 Cargo.toml 的 `[lints]` 里打开 `unwrap_used`、`expect_used`（测试里允许）、`todo`、`dbg_macro`。
2. gewu-app 和 gpui 应用 `#![forbid(unsafe_code)]`：没有 FFI 后，UI 侧不该有 unsafe。
3. 层的边界由编译器保证：gewu-core 和 gewu-app 的依赖树里不出现 gpui，CI 检查。
4. 产品代码里没有 `unwrap`、`eprintln!`、`process::exit`；真正的不变量用带说明的 `expect`，需评审放行。
5. `cargo doc` 无警告；cargo deny、markdownlint 和上面的 Rust 约定（单元测试放同级文件）照常执行。

评审清单：

1. 一个清晰的模型（见上面的通用规则）：快捷键只走 action，缓存只有一套淘汰策略。名不副实的残留（例如 `sorting` 实际指任何后台查询）算缺陷。
2. 每份状态只有一个主人、一个真相源；数据单向流动；模块之间没有循环依赖。
3. 非法状态无法表示：状态机用枚举，不用几个 bool 拼；源行号、视图行号、列 id 用不同的 newtype，防止混用；选区等值用校验过的构造函数，0 行 / 0 列时造不出越界值。
4. 副作用有明确入口：render 纯（见分层与约束）；每个后台任务有持有它的句柄；过时结果不能装上去要靠结构保证（版本号、单一任务槽），不靠「记得检查」。
5. 错误有结构、有归宿：保留 Kind，不吞掉、不 lossy 替换、不只打日志；用户可见错误走同一通道。
6. 抽象只在真实接缝上：不为想象的复杂度提前分层，也不留上帝文件。文件大小是信号，判断标准是「只有一个变化的理由」。
7. 产品代码里没有测试钩子：逻辑写成纯函数用单测覆盖，交互流程用 UI 测试覆盖；不为测试暴露 pub 字段或塞 Probe。测试断言行为和公开契约，不断言实现细节。
8. 性能是有测试的契约：bench 有门槛（见各步验收），回退要么修要么写明理由；影响性能的改动附上测量结果；热路径每帧不分配、不做整列操作。
9. 注释写为什么，不复述代码；公开 API 的 rustdoc 写明契约（输入范围、错误、线程要求）；过时注释算 bug；TODO 当场做掉或转成 issue。
10. 常量有名字和依据（例如 64 行块、64 MB 复制上限）；同一个阈值只有一套，不并存两套。
11. 提交干净：一个提交一件事，删除旧前端、修改 Core 公开 API、重构、功能分开提交；提交说明写清验证了什么（见通用规则中"分清验证过什么"）。

第 3、4 条对本项目最关键：表格加异步查询的 bug 几乎都出在行号混用和过时结果上，用结构挡住比靠评审可靠。

必测边界：0 行、0 列、超大行数（滚动锚点精度）、空字符串和 null、窗口缩到极小、查询失败、过时结果被丢弃。涉及的改动必须覆盖其中相关的项。

评审提问（每个改动合并前回答）：

1. 改动之后，每件事是否仍然只有一种做法，有没有留下旧概念、旧名字？
2. 每个新类型或模块的职责能否用一句话说清，依赖方向对不对？
3. 状态的主人是谁，有没有第二份真相？
4. 副作用和后台任务的生命周期在哪里，过时结果怎么挡住？
5. 测试在哪一层，没有这个修复时它会不会失败？
6. spec、architecture、guide 是否在同一个改动里一起更新？
7. 说"验证过"时，具体是构建、哪些测试、实机还是测量？

## 步骤与验收标准

第 0 步的风险验证可以和第 1–3 步并行，但任何 P0 项不可行就停下重新评估。性能基线：2026-11-22 实测，M4 Mac mini、4K 2x 屏、200 万行 sample.parquet（`gewu-data-gen`，28 列）、表格区域 1280×800、同一个 bench 驱动（慢滚 1 行 / 每帧 37 行 / 随机跳转）。

### 第 0 步：对齐清单与风险验证

做什么：

- 从 docs/spec/（interface.md、search-and-filter.md、data-types.md、file-formats.md）、app/README（在 main 上）和 Swift 测试（GewuKitTests、GewuTests）整理功能对齐清单，每项标 P0（发布必需）/ P1 / P2。
- 对最没把握的几项各做一个最小 spike：Inspector 直方图和散点图；筛选弹窗（facet 列表、范围、文本、缺失值）；打包、签名、公证加自动更新（替代 Sparkle）；自绘表格的无障碍可行性（gpui 的 AccessKit 能否给自定义 canvas 提供虚拟节点）。

验收：

- 清单完成，优先级经 Loong 确认。
- 每个 spike 有结论：可行 / 可行但有代价（写明代价）/ 不可行（写明替代方案）。
- 改动通过「代码质量标准」的 CI 门槛和评审清单。

### 第 1 步：删除旧前端，立工程边界

做什么：

- 一个单独的提交删掉 app/、GewuKit、gewu-uniffi、Xcode 工程和绑定脚本。gewu-uniffi 里的逻辑留到第 2 步搬，删之前先记下清单。
- gpui 应用加入根 workspace，共用一个 Cargo.lock；cargo deny 覆盖它，补齐需要放行的许可证（BSD-2-Clause、ISC、CC0-1.0 等）并注明来源。
- fast 与 pre 的差异收进 platform 模块（目前只有 main.rs 里 `WindowBounds::centered` 两处）。
- 现有窗口自测（interact.rs）和工作流测试（workflow.rs）改写成 gpui-kit test-support 的正式 UI 测试，不要直接删；bench 挪到 examples/；产品代码里去掉 Probe 和只为测试暴露的 pub 字段；去掉 `std::process::exit`。

验收：

- 仓库里不再有 Swift、uniffi、Xcode 工程；从零 `cargo build --release` 就能跑起来。
- CI 三项全绿：fast 下的 test、`--no-default-features` 下的 check、cargo deny。
- 原有 18 项窗口交互检查和工作流检查都以 UI 测试形式跑通；CI 跑不了图形界面的部分有一条本地命令可以一键跑。
- 产品二进制里没有 bench 和测试代码路径。
- 改动通过「代码质量标准」的 CI 门槛和评审清单。

### 第 2 步：gewu-app 与 Core 接口

做什么：

- 新建不依赖 gpui 的 gewu-app，把 gewu-uniffi 里的 app 语义搬过来：apply() 的复用、分析缓存、Find 分页、结构化错误。原型里的 TableQuery / AppliedView 与它合并为一份。
- 把 UI 需要的细粒度 Core API 一次性设计和评审（命名与职责），包括原型已加的 `OrderedView::exact_filter_for_cell` 和 `ColumnHighlight::matches`。
- 滚动锚点、选区、布局计算写成不依赖 gpui 的纯逻辑。

验收：

- gewu-app 和纯逻辑模块在编译上不依赖 gpui。
- 单测覆盖：查询快照的复用与丢弃旧结果、分析缓存、滚动锚点在 100 亿行下的精度、选区边界（含 0 行 / 0 列）。
- Core README 更新了新增的公开 API，每个新接口都有测试。
- 改动通过「代码质量标准」的 CI 门槛和评审清单。

### 第 3 步：表格骨架重构与修硬伤

做什么：

- 按真实接缝拆分 table.rs（约 1040 行）：查询调度、文字缓存、绘制、FindController（通过事件通知上层，不再 `impl TableView`）、交互外壳。不必一次铺满几十个文件。
- render 无副作用（compute_layout 不再改 scroll / scroll_x；不在 render 里创建 Find 会话或 spawn 任务），布局只有一个真相源；后台任务持有 Task 句柄，换视图时在一个入口统一清理。
- 修硬伤（见附录 A）：快捷键严格匹配修饰键并统一走 action + keybinding；0 列保护；遮罩与裁剪；错误和进度汇入统一的状态通道；`sorting` / `last_sort` 改名（现在表示任何后台查询）。
- 无障碍按第 0 步的结论留好接口（可见单元格、表头、选区能提供给无障碍树）。

验收：

- 模块之间没有循环依赖，单个文件不超过约 400 行。
- 第 1 步的 UI 测试全部仍然通过，并新增：Cmd-Shift-C 和 Ctrl-Cmd-F 不被误触、0 列数据集、窗口缩到极小、查询失败后错误的显示与清除。
- 性能不退化：fast、1280×800 下跳转帧 p50 ≤ 4 ms、慢滚 ≤ 2 ms，三种模式都 60 fps、零空白单元格。
- 改动通过「代码质量标准」的 CI 门槛和评审清单。

### 第 4 步：定外观方向并统一 UI 质感

做什么：

- 先由 Loong 定方向：贴近 macOS 原生，还是用 gpui-kit 主题。
- 然后统一做：亮 / 暗模式和强调色、等宽数字、null 样式、表头类型副标题、截断省略号、细线、水平滚动条、列分隔线悬停时的调整列宽光标、表头和滑块的悬停态、拖选到边缘持续滚动、底部状态栏、窗口标题显示文件名、按内容采样计算初始列宽。

验收：

- Loong 在 4K 屏上看过亮、暗两种模式并认可。
- 清单里的界面项逐条打勾；性能仍满足第 3 步的门槛。
- 改动通过「代码质量标准」的 CI 门槛和评审清单。

### 第 5 步：按优先级迁完整功能

做什么：

- 按清单顺序：多列排序 → 筛选弹窗 → 列显隐和重排 → 右键菜单和单元格详情 → 菜单栏、打开文件、拖放、多窗口、最近文件 → 导出对话框和进度 → Inspector（统计、直方图、散点图）→ 其余项。
- 外围控件优先用 gpui-kit 组件。
- 实现无障碍：至少能用 VoiceOver 读到表头、当前单元格和选区。
- 单元格排版结果（ShapedLine）的缓存只在 120 Hz 实测不满足门槛时再做。

验收：

- 清单里所有 P0、P1 项打勾，每项至少有一个 UI 测试或 gewu-app 测试覆盖。
- 改动通过「代码质量标准」的 CI 门槛和评审清单。

### 第 6 步：发布链路与文档

做什么：

- app bundle、图标、签名、公证、自动更新；从 Swift 版迁移偏好设置和最近文件。
- 文档：重写 docs/dev/architecture.md 和 guide.md 里 FFI、GewuKit、App 相关部分（上面「继续适用的规则」是起点）；更新 DEVELOPMENT.md（唯一的命令清单）和 AGENTS.md（只引用、只放 agent 规则）；design/README 表格里把 frontend-redesign.md 标为 superseded，加上本文档；README 的技术栈说明改为纯 Rust。
- 在不同机器上做发布前的性能和资源测试。

验收：

- CI 能产出签名并公证的 .app；在一台干净的 Mac 上能安装、打开文件、自动更新到下一个版本。
- 从 Swift 版升级的用户，偏好设置和最近文件得到保留，或有明确的处理方式。
- 文档里没有过时的 Swift / FFI 描述。
- 改动通过「代码质量标准」的 CI 门槛和评审清单。

## 最终验收标准

以下全部满足，分支才合回 main、替代 Swift 版。

1. 功能：对齐清单所有 P0、P1 项完成；P2 项要么完成，要么 Loong 明确同意推迟。
2. 架构：仓库里没有 FFI、Swift 和 Xcode 工程；Core → gewu-app → gpui 应用单向依赖，只有最外一层依赖 gpui。
3. 正确性：Core、gewu-app、UI 测试在 CI 全绿（fast test + pre check + cargo deny），「代码质量标准」的 CI 门槛全部通过，评审清单没有未解决项。
4. 性能（200 万行，同一个 bench 驱动）：
    - M4、4K 全屏：三种模式稳定 60 fps，零空白单元格，fast 下跳转帧 p95 ≤ 6 ms。
    - 120 Hz MacBook Pro 全屏：跳转帧 p95 ≤ 8.3 ms（尚未测过，必须补）。
    - 一台较慢的机器（例如 M1 基础款）上也是 60 fps。
5. 资源：持续滚动时进程加 WindowServer 的 CPU 不高于 AppKit 基线；静止时 CPU 和 GPU 为 0；内存不超过 AppKit 基线的 110%。
6. 无障碍：VoiceOver 能导航表格，读出表头、当前单元格和选区。
7. 发布：签名、公证、自动更新跑通一个完整版本周期。
8. 文档：architecture、guide、spec、DEVELOPMENT、README 描述的就是新实现；design/ 里有本文档并标明状态。
9. 体验：Loong 在真实数据上用过一段时间，认为可以替代现在的版本。

## 待决定的问题

- 第 0 步先做哪个 spike（建议 Inspector 直方图，或打包加自动更新）。
- 外观方向（第 4 步前）。
- 多个助手可以在本分支上协作，但同一时间只由一个改同一个模块。

## 附录 A：已核实的现状问题（基线 b2056bb）

- render 有副作用：compute_layout 改 scroll 的 viewport 和 scroll_x；查找开着时 render 里调 request_highlights，现场创建 Find 会话并 spawn 后台任务。
- find.rs 写的是 `impl TableView`，与 table.rs 互相依赖。
- 快捷键不检查多余修饰键：("c", cmd) 在有 Shift 时也匹配；("f", cmd) 会匹配 Ctrl-Cmd-F，抢走系统全屏。Cmd-C、Cmd-F、Escape 同时有 key_down 硬匹配和 action 两条路；Cmd-Shift-L / Cmd-Option-L 只有硬编码。
- selection、斑马纹没进 body 遮罩（目前被后画的表头和行号列盖住，看不出来，但脆弱）。
- workflow.rs 用 `std::process::exit`；interact.rs 硬编码 28 列和 "uuid"。
- 两个 Cargo.lock 已漂移（zstd-sys 2.1.2 对 2.1.3）；gpui/ 是独立 workspace，cargo deny 扫不到。
- data.rs：CACHE_BLOCKS 与 CACHE_BYTES 两套淘汰阈值并存；`use` 夹在常量中间。
- Probe（bench 用）留在产品代码里。

## 附录 B：参考数据（2026-11-22，M4 Mac mini）

- 帧时间（render→paint，200 万行，fast，1280×800）：慢滚 p50 1.6 ms、每帧 37 行 3.4 ms、随机跳转 3.5 ms；近全屏 4K 跳转约 4.8 ms；pre 约为 fast 的两倍。全部 60 fps、零空白。
- AppKit 基线：慢滚主线程 1.1 ms；跳转帧 80–98% 显示占位符（gewu 的异步 64 行块读取导致，不是 AppKit 限制）。
- 资源（进程 + WindowServer CPU）：持续滚动 AppKit 88–137%，gpui-fast 31–39%；静止 gpui 0%。GPU 三者都低（整机 ≤ 约 11%），基本打平。内存三者约 650–740 MB。
- 对照构建：~/tmp/gewu-bench（AppKit 基线，`GEWU_SCROLL_BENCH=1` 启用滚动驱动）、~/tmp/gpui-proto（gpui-kit DataTable 对照）、~/tmp/archive/gewu-gpui-2026-11-22（旧原型二进制、测量脚本 measure_usage.py / gpu_baseline.py、日志）。
- 已知 GPUI 坑：Pixels 是 f32，偏移越大精度越差（200 万行底部的 ulp 是 4 px，0.5–1.5 px 的滚动增量完全丢失），所以表格用 u64 顶行 + f64 行内偏移；macOS 平台层丢掉 momentumPhase；无头测试的帧时间没有意义（NoopTextSystem）；Mac mini 测试需 `caffeinate -u -d` 且窗口处于激活状态。
