# Gewu 视觉设计规范

本文档确立 Gewu 的视觉定位、排版间距与层级参数、固定语义颜色 Token 体系、控件状态矩阵、可选高对比度模式与非颜色提示规则，以及表格、查找、图表和浮层的专属细则。依据：[产品界面规格](../../spec/interface.md)、[查找与筛选规格](../../spec/search-and-filter.md) 与 [theme.rs](../../../gpui/src/theme.rs)。

文档定位与范围说明：`docs/spec/` 只描述用户可见的功能与交互行为；本规范涵盖色值、Token、几何与控件状态等实现级约束，与产品交互规格配套使用。范围筛选上界显示方式（是否改开区间）与缺失值默认策略归属 [search-and-filter.md](../../spec/search-and-filter.md)，不在本规范内决定。

---

## 一、视觉定位与可检查标准

Gewu 不过分追求模仿系统原生控件的外观细节，但绝不能像十年前、二十年前的老旧桌面软件，也不能像网页端后台；整体精致度对标 **Zed、Linear、TablePlus**。具体以三条可检查标准为准：

- **不像老软件**：不用渐变、斜面、内阴影、粗黑边框；区域分隔使用半透明细线；高饱和色彩仅出现在强调色、查找高亮和图表数据上，其余保持克制的中性灰阶；同一种控件全应用只有唯一一套实现，尺寸、圆角、间距严格统一。
- **不像网页**：不用亮色底配黑字的按钮，不用大内边距、胶囊按钮堆叠、大圆角松散卡片；保持紧凑的桌面信息密度（标准控件高 `26–28pt`，正文 `13pt`）；键盘操作、焦点环、右键菜单完整，不依赖仅悬停才出现的操作。
- **精美**：空间层级只靠字号字重（约三级）、灰阶前景（正文 / 次要 / 禁用）与表面高度（`canvas` / `surface` / `elevated`）建立，不靠花哨颜色和重边框；细线对齐物理像素（`1 / scale_factor`）；浮层拥有清晰的空间分离（`elevated` 表面 + 细边 + 柔和投影）；查找高亮与选区不压过数据阅读本身。

---

## 二、排版、间距、圆角、层级投影与基础动效

### 2.1 字体与字号字重（三级）

- **正文字级（`13pt` Regular）**：表格单元格、普通按钮、输入框、下拉框与标签正文（表格数值、行号与界面计数统一启用 `tnum` 等宽数字）。
- **结构标题级（`12pt` Medium / 弹层标题 `13pt` Semibold）**：表头列名、检查器与浮层区块小标题、模态弹窗标题。
- **辅助次级（`10.5–11pt` Regular）**：表头列类型、行号、直方图与散点图坐标轴刻度、副标题与条件说明（刻度与计数启用 `tnum`）。
- **专有文本字体**：单元格纯值预览浮层沿用表格单元格同款字体（`13pt` + `tnum`，`null` 保持斜体弱化）；错误页 `Technical details` 诊断框使用 `12pt` 等宽字体。

### 2.2 间距、控件高度与圆角标尺

- **间距标尺**：全应用严格只使用 **`4 / 8 / 12 / 16 / 24pt`**（面板内边距与卡片间距以 `12pt`、`16pt` 为主，行内控件间距用 `4pt`、`8pt`）。
- **表格行高**：支持 `24 / 28 / 32pt` 三档，与 [interface.md](../../spec/interface.md) 一致。
- **标准控件高度**：主/次/危险按钮、单行输入框、下拉选择器与分段控件统一 **`26–28pt`**，同一水平排内的控件必须严格上下齐平（同步修复标题栏 `Find in table` 框比侧栏按钮矮、排序行 `Asc | Desc` 比下拉框矮、范围筛选 `Minimum` / `Maximum` 输入框偏胖的问题）。图标按钮统一 **`22×22pt`**；右键菜单与下拉菜单项保持 **`24–26pt`** 紧凑桌面行高。
- **圆角**：常规按钮、输入框、下拉框、分段外槽为 **`6pt`**；分段选中滑块为 **`4pt`**；浮层、菜单与模态弹窗为 **`10pt`**；开关胶囊与占比条使用 **`rounded_full`**。

### 2.3 层级高度与投影参数（阴影仅用于浮层）

- **浅色模式 `elevated` 浮层**：表面为 `#FFFFFF`，边框 `1px` `elevated_border`，配双层柔和投影：`offset(0, 4) blur 16 #000000@12%` 叠加 `offset(0, 1) blur 3 #000000@6%`。
- **暗色模式 `elevated` 浮层**：主要依靠表面明度逐级抬升（`elevated #2C2C2E` > `surface #252527` > `canvas #1E1E1E`）与顶部/外缘细微高光 `elevated_border`（`#FFFFFF@13%`）建立立体感，辅以克制的深色环境投影 `offset(0, 6) blur 20 #000000@36%`。
- **模态遮罩（`backdrop`）**：亮色 `#000000@24%`，暗色 `#000000@46%`。非浮层控件（主表、侧栏、分段滑块微调之外的区域）不使用大投影。

### 2.4 焦点环、滚动条、图标与动效

- **焦点环**：全应用统一 `2px` 宽度、外扩 `1px`，使用 `focus_ring`；所有可聚焦控件共享同一几何规格。
- **滚动条**：轻量半透明覆盖式，不挤压主表或侧栏右侧留白。
- **图标**：单色线性风格，`14–16pt`，统一使用 `foreground` 或 `muted_foreground`。
- **动效**：仅在弹层显隐与检查器侧栏展开/收起时使用 `100–150ms` 短促缓动（尊重系统“减少动态效果”设置）；表格滚动、单元格选区移动与查找结果跳转保持零动画即时响应。

---

## 三、语义颜色 Token 与 `Default` 默认主题色表

- **强调色由主题固定，不跟随系统**：默认主题命名为 **`Default`**；移除 [platform.rs](../../../gpui/src/platform.rs) 中读取 `NSColor::controlAccentColor()` 的逻辑，`document_colors` 不再接收外部强调色参数，强调色固定为亮色 `#007AFF`、暗色 `#0A84FF`。
- **前景色按角色成对写死，不在运行时计算**：每组实心填充色与其前景成对声明（`accent` / `accent_foreground`、`danger_fill` / `danger_fill_foreground`），组件只读取不计算；彻底移除 [theme.rs](../../../gpui/src/theme.rs) 中的 `contrasting_ink`。`Default` 主题中强调色与危险色的实心填充上一律使用纯白字、纯白勾；淡色叠底（选区、查找命中）和中性控件一律使用 `foreground`。
- **不以对比度数值作为默认主题目标**：下表颜色基于 AppKit 语义色与桌面工具经验选定作为参考起点，最终以亮/暗实拍截图肉眼校准为准；全部颜色字面量仅允许出现在 [theme.rs](../../../gpui/src/theme.rs)（与主题单测）内。

以下列出本轮先落地的实际使用的 **34 个语义颜色 Token**（`disabled_opacity` 归入第四节状态规则）：

| 分组 | Token 名称 | `Default Light` | `Default Dark` | 参考依据与层级说明 |
| :--- | :--- | :--- | :--- | :--- |
| 表面层级 | `canvas` | `#FFFFFF` | `#1E1E1E` | 对齐 `windowBackgroundColor`，主表与窗口底色 |
| 表面层级 | `stripe` | `#000000@3.5%` | `#FFFFFF@4%` | 参考 `alternatingContentBackgroundColors[1]` 微调，铺满表体行宽 |
| 表面层级 | `surface` | `#F6F6F6` | `#252527` | 表头、左侧行号槽、检查器与状态栏，略区分于 `canvas` |
| 表面层级 | `elevated` | `#FFFFFF` | `#2C2C2E` | 菜单、Tooltip、筛选浮层与模态弹窗，暗色明确高于 `surface` |
| 表面层级 | `backdrop` | `#000000@24%` | `#000000@46%` | 模态对话框背景压暗遮罩 |
| 边线网格 | `border` | `#000000@11%` | `#FFFFFF@12%` | 对齐 `separatorColor`，面板分割线与控件常规边框 |
| 边线网格 | `grid` | `#000000@7%` | `#FFFFFF@7%` | 数据表列分割线，弱于面板 `border` |
| 边线网格 | `elevated_border` | `#000000@12%` | `#FFFFFF@13%` | 浮层与弹窗细边框（暗色提供边缘微弱高光） |
| 文字前景 | `foreground` | `#262626` | `#E5E5E7` | 对齐 `labelColor`，正文、单元格数值、按钮主文字 |
| 文字前景 | `muted_foreground` | `#68686D` | `#98989D` | 对齐 `secondaryLabelColor`，列类型、行号、斜体 `null`、说明标签 |
| 通用控件 | `segment_track` | `#000000@6%` | `#000000@28%` | 分段控件凹槽底色（浅色配极淡 `border` 轮廓避免悬浮感） |
| 通用控件 | `segment_thumb` | `#FFFFFF` | `#FFFFFF@16%` | 分段控件选中滑块（暗色参考 `controlColor`，严格亮于 `segment_track`） |
| 通用控件 | `switch_track_off` | `#000000@16%` | `#FFFFFF@20%` | 开关关闭态中性轨道，不再复用 `muted_foreground` |
| 通用控件 | `switch_thumb` | `#FFFFFF` | `#FFFFFF` | 开关圆形拨块，全状态保持纯白实体 |
| 通用控件 | `button_secondary` | `#000000@6%` | `#FFFFFF@11%` | 全应用唯一的次级按钮默认底色 |
| 通用控件 | `button_secondary_hover` | `#000000@10%` | `#FFFFFF@16%` | 次级按钮与菜单项悬停底色 |
| 强调状态 | `accent` | `#007AFF` | `#0A84FF` | 主题固定强调色，用于主按钮、复选框、开态开关与激活漏斗 |
| 强调状态 | `accent_foreground` | `#FFFFFF` | `#FFFFFF` | 成对写死的纯白文字与纯白复选框对勾 |
| 强调状态 | `accent_hover` | `#006CE6` | `#2693FF` | 实心强调色按钮不透明悬停底色 |
| 强调状态 | `selection_active` | `#007AFF@16%` | `#0A84FF@22%` | 激活窗口选区底色，同步映射对应表头与行号栏高亮 |
| 强调状态 | `selection_inactive` | `#DCDCDC@55%` | `#FFFFFF@10%` | 对齐 `unemphasizedSelectedContentBackgroundColor`，窗口失焦中性选区 |
| 强调状态 | `focus_ring` | `#007AFF` | `#0A84FF` | 键盘焦点环与当前单元格内框（失焦退化为 `border` 加深中性灰） |
| 强调状态 | `find_span` | `#E5B84B@28%` | `#E5B84B@26%` | 单层查找命中子串暖琥珀底色（具体浓度以 `03`/`31b` 实拍为准） |
| 强调状态 | `find_span_current` | `#E5B84B@48%` | `#E5B84B@44%` | 当前查找命中子串，明确深于普通命中一级 |
| 强调状态 | `find_current_border` | `#C48F00@78%` | `#F0C24B@78%` | 当前查找命中单元格的 `1.5px` 内描边 |
| 强调状态 | `danger_text` | `#C92A2A` | `#FF6961` | 报错与警示提示文字 |
| 强调状态 | `danger_fill` | `#D9383A` | `#D9383A` | 实心危险确认按钮（`Hide`）底色，与 `danger_text` 分离 |
| 强调状态 | `danger_fill_foreground` | `#FFFFFF` | `#FFFFFF` | 实心危险按钮成对写死的纯白文字 |
| 图表数据 | `chart_primary` | `#007AFF@75%` | `#0A84FF@82%` | 直方图选中柱、散点与 `Common values` 占比条 |
| 图表数据 | `chart_primary_hover` | `#007AFF` | `#0A84FF` | 图表柱与散点悬停/高光色 |
| 图表数据 | `chart_muted` | `#000000@20%` | `#FFFFFF@22%` | 范围筛选直方图未选中柱子的中性灰 |
| 图表数据 | `brush_fill` | `#007AFF@14%` | `#0A84FF@18%` | 范围直方图刷选区间底色（配合 `1px` `accent` 描边） |
| 图表数据 | `chart_grid` | `#000000@7%` | `#FFFFFF@8%` | 直方图与散点图参考网格线 |
| 图表数据 | `bar_track` | `#000000@7%` | `#FFFFFF@10%` | `Common values` 100% 宽度圆角底轨 |

框架兼容说明：在 [theme.rs](../../../gpui/src/theme.rs) 中将核心通用字段同步映射到 `gpui_kit::base::ColorTokens`（`background`、`surface`、`foreground`、`muted_foreground`、`border`、`primary`、`primary_foreground`、`secondary`、`destructive`、`ring`、`selection`），供内置基础组件消费；完整 34 项 `GewuPalette` 作为全局扩展供 Gewu 表格与面板直接读取。

---

## 四、控件状态表

全应用所有禁用态控件统一附加 **`disabled_opacity = 0.45`** 与 `cursor_default`，且禁用时不响应悬停或按下变化；同一类控件全应用共用唯一组件实现：

| 控件类型 | 默认 | 悬停 | 按下 | 禁用 | 焦点 | 窗口失焦 |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| 主按钮 | `accent` 底 + `accent_foreground` 白字 | `accent_hover`（不透明） | `accent` 微压暗（不透明） | `opacity(0.45)` + 无 hover | `focus_ring` 外框 | 底色不变或按原生略弱化、文字保持可读 |
| 次级按钮 | `button_secondary` 底 + `foreground` 字（全应用唯一样式，统一替换白底描边按钮） | `button_secondary_hover` | 底色再深一阶（`+4%` alpha） | `opacity(0.45)` + 无 hover | `focus_ring` 外框 | 保持中性外观 |
| 危险按钮 | `danger_fill` 底 + `danger_fill_foreground` 白字 | 不透明略压暗 `#C32E30` | 再压暗 `#AD2628` | `opacity(0.45)` + 无 hover | `focus_ring` 外框 | 保持可读 |
| 分段控件（全应用 5 处共用唯一组件） | 槽 `segment_track` + 选块 `segment_thumb` + `foreground` 字 | 未选项底轻叠 `button_secondary_hover` | 未选项微沉降反馈 | `opacity(0.45)` | 外槽现 `focus_ring` | 保持中性层次，不随强调色变化 |
| 复选框 | 未选 `canvas` 底 + `muted_foreground` 框；已选 `accent` 底 + `accent_foreground` 白勾 | 框边略加深 / `accent_hover` | 轻微缩小或底色略压暗 | `opacity(0.45)` | 勾选框外 `focus_ring` | 已选可保持柔和中性或中性灰边 |
| 开关 | 关闭 `switch_track_off`；开启 `accent`；圆点恒为 `switch_thumb` | 轨道微加深 | 轨道轻压暗 | `opacity(0.45)` | 胶囊外 `focus_ring` | 退为中性轨道色，圆点位置不变 |
| 输入框 / 下拉框（`26–28pt` 齐平） | `canvas` 或 `surface` 底 + `1px` `border` + `foreground`（占位符 `muted_foreground`） | 边框略加深至 `#000000@20%` / `#FFFFFF@22%` | 下拉触发保持悬停加深 | `opacity(0.45)` | `focus_ring` `2px` 外框 | 焦点框消退或退回 `border` |
| 右键 / 下拉菜单项（`elevated` 容器，行高 `24–26pt`） | 透明底 + `foreground` 字（快捷键 `muted_foreground`） | `button_secondary_hover` 圆角高亮条 | 底色微加深 | `muted_foreground` + `opacity(0.45)` | 键盘高亮同悬停条 | 菜单关闭 |

---

## 五、高对比度模式与不只靠颜色区分（色盲友好）

### 5.1 高对比度模式（可选，非默认）

- **三档设置**：Preferences 中提供高对比度开关三档——**`跟随系统（默认）/ 关 / 开`**；当设为“跟随系统”时，在 [platform.rs](../../../gpui/src/platform.rs) 读取 macOS 系统的“提高对比度”（`NSWorkspace.shared.accessibilityDisplayShouldIncreaseContrast`）状态。
- **实现架构**：高对比度模式是**同一套 `GewuPalette` Token 的另一组独立取值表**，组件渲染代码内部不写 `if high_contrast` 分支。开启后在色表中同步加深：`border`、`grid`、`elevated_border`、`muted_foreground`、`find_current_border`，并适当压深 `accent` 与 `danger_fill`。
- **对比度测试归属**：`>= 4.5 : 1` 的 WCAG 对比度自动化断言**只针对高对比度模式运行**。

### 5.2 不只靠颜色区分（色盲友好，默认外观即生效）

即使在普通默认模式下，所有关键状态也必须具备非颜色视觉线索：

- **查找当前命中**：除子串底色外，额外绘制 `1.5px` `find_current_border` 内描边，不单靠背景区分。
- **表格当前单元格**：带清晰的 `1.5–2px` 单元格内描边，选区边缘清晰。
- **错误与警示**：报错提示和警示文案旁附带状态图标，不只靠 `danger_text` 红色字传达错误。
- **图表刷选与选中**：直方图刷选范围除 `brush_fill` 与 `chart_primary` 对 `chart_muted` 的区别外，在选区左右与顶部边界加 `1px` `accent` 描边。
- **系统增强**：当 macOS 开启“区分时不只用颜色”（`accessibilityDisplayShouldDifferentiateWithoutColor`）或高对比度模式时，进一步加粗当前选中框、为直方图非选柱/刷选区补充纹理或更明确的边框区分。

---

## 六、表格、查找、图表与浮层专属细则

### 6.1 表格专属规范

- **表头两行**：列名 `12pt` Medium（`foreground`），数据类型 `10.5pt` Regular（`muted_foreground`）。
- **排序与筛选图标**：排序方向箭头（`▲`/`▼`）与多列优先级数字常态清晰可见（`foreground`）；未激活筛选的列漏斗在悬停或列选中时显现（`muted_foreground`），已应用筛选的列常驻高亮为 `accent`。
- **选区与联动**：当前活动单元格绘制内描边；选区覆盖的所有行号格与列头同步叠加选中高亮；表格保留 `stripe` 斑马纹（铺满表体行宽）与 `surface` 左侧行号栏。
- **空值与列宽手柄**：`null` 使用 `muted_foreground` 斜体；列宽拖拽手柄悬停时显示清晰细线标识；窗口失焦时选区与当前单元格框全部退化为 `selection_inactive` 与中性灰框。

### 6.2 查找高亮（单层设计 + 截断兜底 + 绘制层级）

- **单层高亮规则**：仅对命中的文本子串铺 `find_span` 底色；**单元格整体不再铺底色，也不绘制底部通栏横线**。当前命中单元格将其子串加深一级为 `find_span_current`，并在单元格四周绘制 `1.5px` `find_current_border` 内框（文本维持原有 `foreground`，不另配刺眼高对比字色）。
- **窄列截断兜底**：当某命中单元格的匹配子串全部落在列宽截断 `…` 之后（[find_style.rs](../../../gpui/src/table/find_style.rs) 可见区内无匹配切片）时，自动给末尾的 `…` 铺 `find_span`（当前命中时 `find_span_current`），避免截断命中在界面上静默隐身。
- **重叠绘制层级（从底到顶）**：`选区底（selection）` → `命中子串（find_span / find_span_current）` → `当前命中框（find_current_border）` → `当前单元格框（focus_ring）`。
- **验收标准**：200 万行大表快速滚动时命中清晰可读；`31b` 整列全命中时清爽透气，不糊成一整条黄色色带；多命中同屏时能一眼分辨当前命中项。

### 6.3 图表与侧栏分布条

- **范围筛选直方图**：未选中柱用 `chart_muted`，选中区间内柱用 `chart_primary`，区间背景铺 `brush_fill` 并加 `1px` `accent` 边界描边；直方图柱间距统一 `1px`。
- **散点图与坐标轴**：散点使用 `chart_primary`（悬停 `chart_primary_hover`），选中点绘制高辨识度外描边环；坐标轴网格用 `chart_grid`，刻度标签统一 `11pt`、`muted_foreground`、`tnum`。
- **侧栏 `Common values`**：占比蓝条（`chart_primary`）包裹在 100% 宽度的 `bar_track` 胶囊轨道中，一眼可对照所占比例。

### 6.4 浮层、菜单与对话框收口

- **极简单元格纯值预览浮层（原 `Cell details`）**：去掉 `"Cell details"` 粗标题以及 `Remarks` 等冗余列名（单元格已在原位高亮指示），弹开直接展示完整单元格值本身；**字体与表格单元格完全一致**（`13pt` + `tnum`，`null` 斜体弱化）；宽高紧贴内容紧凑自适应（短值不撑出大块留白，长值在最大宽高限制内滚动）；只提供轻巧的 `Copy` 操作，按 `Esc` 或点击外部即关闭。
- **错误页 `Technical details`**：放入 `12pt` 内边距、`6pt` 圆角的统一浅底（`surface`）细边（`border`）可滚动诊断框中，正文使用 `12pt` 等宽字体。
- **暗色右键菜单（修复 `dark/18`）**：上下文菜单容器统一采用 `elevated` + `elevated_border` + 柔和投影，菜单项行高收紧至 `24–26pt`。
- **次级按钮全应用唯一化（修复 `19b`）**：`Visible columns` 顶部的 `Show matching` / `Hide matching` 与底部按钮统一使用标准 `button_secondary`，取消白底细边异形按钮。
- **`Active filters` 筛选摘要规整（修复 `16b`）**：每条条件行左侧放置定位图标与列名，右侧将 `Clear` 按钮右对齐推至行尾；条件详情使用 `11pt` `muted_foreground` 紧贴列名下方对齐排列。
- **`File Info` 顺手微调（低优先级，不强求）**：按 `12–16pt` 间距标尺适当收紧 [shell/info.rs](../../../gpui/src/shell/info.rs) 标签列与数值列之间的宽空白。

---

## 七、自动化检查与验收清单

- **源码颜色字面量静态检查**：执行 `grep` 确认除 [theme.rs](../../../gpui/src/theme.rs)（与 [theme_tests.rs](../../../gpui/src/theme_tests.rs)）外，整个 `gpui/src/` 不存在任何硬编码颜色值（`rgb(`、`rgba(`、`hsla(`、`white()`、`black()`、`0x...`）。
- **主题单测重写（[theme_tests.rs](../../../gpui/src/theme_tests.rs)）**：
  - `Default` 默认主题**只测结构不变量**（不跑 `4.5 : 1` 断言）：暗色 `luminance(segment_thumb) > luminance(segment_track)`、`luminance(elevated) > luminance(surface) > luminance(canvas)`、`accent_foreground == white()`、`danger_fill_foreground == white()`、暗色 `danger_fill != danger_text`、`accent_hover` 保持不透明（`a == 1.0`）。
  - **高对比度模式**单独运行完整的阅读文本与控件 `>= 4.5 : 1` 对比度断言。
- **完整验收截图清单**：
  - 标准模式：`Light` / `Dark` × 前台活动窗口 & 非活动窗口 × `1×` / `2×` 分辨率，至少覆盖 `03`（单层查找高亮）、`31b`（200 万行整列命中）、`10`（主按钮与复选框白字白勾）、`20`（危险按钮红底白字）、`01`/`21`/`29`（深色分段控件极性与同排控件齐平）、`Log Y` 开与关两态、`18`/`23`（右键菜单抬升表面）、`19b`（次级按钮统一）、`16b`（筛选摘要对齐）、`24`（极简纯值浮层）、`30b`（错误页诊断框）以及 `14`（暗色筛选弹层与范围输入框高度）。
  - 高对比度模式：另拍一套亮/暗对照截图。

---

## 八、实施顺序与后续扩展

1. **阶段 1（本轮首要）**：落地 `GewuPalette` 34 个颜色 Token 与 `Default` 亮/暗主题（固定 `#007AFF` / `#0A84FF`，移除 `controlAccentColor` 读取与 `contrasting_ink`），按第四节、第六节完成全部控件状态统一与 V1–V7 视觉收口，通过结构不变量测试、`grep` 颜色检查及全套截图验收。
2. **阶段 2（本轮次要）**：在同套 Token 架构上落地三档高对比度模式（`跟随系统（默认）/ 关 / 开` + `>= 4.5 : 1` 断言）与非颜色提示（错误图标、图表刷选描边、系统“区分时不只用颜色”增强），并补拍高对比度亮/暗截图。
3. **以后（不在本轮范围）**：基于同一套 Token 扩展 `Catppuccin`（`Latte` / `Mocha`，并在其主题表内静态写死柔粉彩深字前景）以及偏好设置里的主题/外观切换器，不在本轮分散实现焦点。
