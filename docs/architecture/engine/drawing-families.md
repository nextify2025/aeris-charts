# 绘图族与扩展流程

[文档导航](../../README.md) · [架构总览](../../Architecture.md)

各族共享[绘图状态与历史](drawings.md)、[输入控制器](input.md)和[文本会话](drawing-text.md)。B8 是其在扩展计划中的批次名称，不是另一套运行时。

- [目录与共享钩子](#目录与共享钩子)
- [部件词汇与转换](#部件词汇与转换)
- [上游种类选项](#上游种类选项)
- [族选项](#族选项)
- [旧版工具名与文档迁移](#旧版工具名与文档迁移)
- [系列数据读取方](#系列数据读取方)
- [锚定文本与图标戳](#锚定文本与图标戳)
- [各族语义](#各族语义)
- [Wire id 与注册表](#wire-id-与注册表)
- [添加绘图族](#添加绘图族)

## 目录与共享钩子

B8 目录即 AerisTerminal 上游的目录：85 个种类，wire id 为 `0..=84`，按上游的枚举顺序排列（从趋势线到锚定 VWAP，接着是价格范围、日期范围与日期价格范围，然后从射线到柱形态），列于 `DRAWING_TOOL_SPECS`（`drawings/tools.rs`）中。每个种类恰好有一个渲染器，由其规格的 `family` 决定是哪一个。除三个测量工具外，每个上游种类都是 `family: None`，对这些种类而言，上游的实现是唯一的实现：目录规格、`drawings/geometry.rs` 中的主体解析器（`DrawingBodyGeometry`）、`frame/drawings.rs` 中的转换分支，以及对同一已解析主体的精确命中测试，详见[绘图几何](drawings.md#派生运行时与几何)。恰好有十个种类设置 `family: Some(..)` 并使用下文的族路径：上游没有的七个自有线工具（`horizontal_segment` 240、`vertical_ray` 241、`vertical_segment` 242、`price_line` 243、`price_channel` 244、`simple_tag` 245 与 `simple_annotation` 246），以及测量工具 `price_range` 13、`date_range` 14 与 `date_price_range` 15，它们保留自有线的实现（网格吸附、统计、坐标轴标签以及 Shift 点击快速测量；见[测量工具](drawings.md#测量工具与瞬态测量)）。两条线都实现过的工具，其自有线族渲染器已在两条线合并时退役：上游转换所不具备的渲染器附加功能列在 `drawings/tools.rs` 顶部的 `ponytail:` 注记中，其选项键仍会被存储但不起作用（见[族选项](#族选项)）。其中四项改为在上游的转换之上重新实现：通道的 `extend_left` 与 `extend_right` 会把两条线及其填充一直延伸到窗格边缘（`geometry.rs` 中的 `extend_channel_line`）；标注框在其尖端与框上各保留一个手柄（`CALLOUT` 设置 `DrawingHandleMode::Anchors`）；实线荧光笔转换为其展开路径周围管状区域的一次半透明 `BandFill`（`shape::tube_ribbon`），因此自身重叠的描边在 GPU 执行器上与在 Canvas2D 上一样，每个像素只混合一次（虚线荧光笔保留描边）；没有拟合结果的回归趋势线会绘制其锚点之间的虚线段（`RegressionWindow`，见[系列数据读取方](#系列数据读取方)）。`kinds/fibonacci.rs`、`kinds/pitchforks_gann.rs`、`kinds/patterns_elliott_cycles.rs` 与 `kinds/shapes.rs` 只保留其公共选项类型与自有线的旧版默认值表；`kinds/lines.rs`、`kinds/channels.rs` 与 `kinds/projection_annotations.rs` 保留自有线工具与范围工具，以及下文所述的数据读取方（[系列数据读取方](#系列数据读取方)）、旧版默认值与内置图标字形（[锚定文本与图标戳](#锚定文本与图标戳)）。

上游的目录常量显式设置 `text_layout`。`line_spec` 与 `channel_spec` 种类（射线、延长线、信息线、角度线与箭头线；平行通道、平顶通道、平底通道与不相交通道）保持 `Segment`：标签跟随前两个锚点、随之旋转并取其描边颜色，居中标签会拆分描边。`shape_spec` 以及由它构建的每个种类（回归趋势线、从旋转矩形到双曲线、斐波那契、叉形线、形态、艾略特波浪、周期、标记、价格标签、图标戳、江恩、投影、预测与柱形态）都使用 `Box`，文本标注、折线与荧光笔同样如此，十字线则使用 `Box` 并带有水平线的 `axis_price_label` 标签。`Drawing::new` 把趋势线、标注框与价格便签（上游的规则）以及布局为 `Segment` 的族工具的标签对齐默认设为右/上；其他每个种类都从居中开始。

族路径服务于这十个族工具。每个族模块包含其工具规格、一份被这些规格引用的 `static FAMILY: DrawingFamily` 钩子表、其类型化选项块及其测试（`kinds/<family>/tests.rs`）。钩子表是封闭的编译期表，而不是插件注册表。`DrawingFamily::new` 接收两个必需的钩子 `build_parts` 与 `kind_options`，并让每个可选钩子都从一个中性默认值开始，族在其 `static` 初始化器中通过赋值覆盖它：`apply_defaults`（由 `Drawing::new` 应用的种类默认值，因此创建、模板、恢复、粘贴与 schema 默认值保持一致）、`decoration_extent`（为锚点之外的框与标签提供的保守 CSS-px 剔除余量）、`extend_schema`（追加在通用描述符之后的 `tool_options.*` 描述符）、`owns_labels`（该族自行渲染通用的 `labels`）、`owns_text`（该族把通用的 `text` 布局在自己的部件中，因此通用文本遍历会跳过它）、`partial_preview`（放置预览从第二个锚点起即可解析部件），以及 `handles`。`handles` 编辑 `drawings/handles.rs` 依据规格的手柄模式以媒体像素构建的手柄集合（把某个手柄移到派生几何上，例如价格通道的第三个手柄位于其第二条线上，或删除某个手柄），而选中手柄的绘制、放置预览（仅限已放置锚点的手柄）、手柄命中测试、键盘手柄循环与拖动起始，全都读取这同一个集合。每个拖动采样（无论是指针拖动还是键盘微调）都会运行通用的部件拖动：锚点经时间吸附、磁吸与拉直后重新锚定，或本体平移。自有线的 `drag` 钩子、其 `HandleDrag` 采样以及 `drawing_anchor_at` 转换只服务于已退役的叉形线、固定江恩方形与旋转矩形渲染器的派生手柄，已随它们一并删除；`DrawingDragPart::Handle` 与 `DrawingDrag::handle_px` 仍保留，`drawings.rs` 中有一条 `ponytail:` 延期说明，待第一个派生手柄的族出现时再重新加入 `drag` 钩子。仅由已退役的族使用的钩子（`paint_bounds`、带 `FamilyBounds` 的 `bounds`、`reads_series_data`、`on_create`、`pane_anchored`、`reveals_on_focus`、`rescale_price_options`、`close_placement` 与 `text_box`）已连同其引擎调用点一并删除。钩子在帧构建与命中测试期间、绘图运行时缓存处于借用状态时运行，因此它们只读取引擎，绝不调用候选查询或缓存的锚点几何访问器。规格字段取代共享代码中逐种类的检查：`text_layout`、`axis_price_label`（水平线的坐标轴标签）、`axis_tag_text`（当绘图带有文本时，该标签显示该绘图的 `text` 而非价格，且该文本不会绘制在图表上，与简单标签的做法一致）、`anchor_link`（见[线条](#线条)）以及 `grid_snap`（测量工具与仓位工具）。

## 部件词汇与转换

一个绘图族会在调用方的空间（渲染时为位图 px，命中测试时为媒体 px）中，把一个绘图解析为 `drawings/parts.rs` 中共享的部件词汇：抗锯齿描边、清晰的整像素水平线与垂直线、成对链之间的带状填充（凸多边形通过 `fill_convex`）、圆盘，以及由同一个 `PartLabel::layout` 排版的带框文本块。派生的逻辑/价格点通过 `PartContext::point_px` 映射到该空间，该函数应用帧各自独立的水平与垂直位图比例；帧构建会以 debug 断言确认它能复现每一个锚点。帧构建在 `frame/drawings.rs` 中把部件转换为现有的 `Prim`（`build_family_prims`；每一段描边都经过 `push_clipped_stroke`，它用 `shape::clip_polyline_to_rect` 把描边裁剪到按描边延伸量外扩后的窗格范围内，因此无论几何延伸多远，工作量与坐标都保持有界，并且和引擎拥有的所有路径描边一样，通过 `push_line_stroke` 把虚线或点线描边段拆分为实线的短划段，因为 WebGPU 细分器没有虚线概念；被裁剪的部件保留未裁剪描边段的虚线相位，因此平移时虚线绝不会偏移），而精确命中测试检测的是同样的部件（`DrawingParts::hit`，其中虚线描边仍是一个连续的主体），所以各族工具不能分叉执行器行为，绘制出的形状与可交互的形状也不会漂移。[锚定文本与图标戳](#锚定文本与图标戳)中所述的内置图标字形回退也经由同一路径转换。

族工具放置期间，一旦每个锚点都已放置或处于预览中（设置 `partial_preview` 时，从第二个锚点起），帧的待定路径就会构建该工具自身的部件；在此之前，具有三个或更多锚点的工具会把已放置的锚点与指针连成一条以绘图描边绘制的引导折线，该折线经 `push_clipped_stroke` 转换，并在已放置的锚点上显示手柄，因此每次点击都会留下可见的笔迹。共享辅助函数位于各自的所有者处：纯几何（线段延长、中点以及到窗格的裁剪、射线裁剪、平行偏移、折线、多边形与带状命中判定，以及 `Rect` 的外扩、求交与包围盒）位于 `aeris_charts_render::shape`；线帽（`capped_segment` 就是两点的 `capped_polyline`，`cap_radius` 决定每个圆盘与箭头的尺寸）、箭头修剪、标签排版、统计框（`PartContext::stats_label`，使用共享的 `STATS_*` 间距、内边距与 alpha，以及 `text_on` 选出的黑色或白色文字）以及填充约定（`PartContext::fills_hit`：区域填充仅在绘图被选中时才是主体目标，与矩形内部相同）位于 `parts.rs`；颜色解析位于契约类型上（`Drawing::stroke_color` 带规范的主色回退，以及 `Drawing::fill_or_wash`）；属性描述符构建器 `drawing_contract::descriptor`；引擎格式化的测量文本（价格经绘图比例尺的格式化器、百分比、tick 数、柱数、时间范围、时长、屏幕角度、距离）以及文本与统计字形大小（`drawing_text_size`、`drawing_stats_size`）位于 `drawings/stats.rs`；以及位于 `drawings/handles.rs` 的唯一可编辑手柄集合。

设置了 `extend_left` 或 `extend_right` 的绘图使用无界的语义边界，因此锚点滚出视野时，延长线仍保持可见且可命中；而关闭了延长的线，则与任何有限线段一样被剔除。

## 上游种类选项

上游种类把其选项保留为 `Drawing` 上的扁平字段，由绘图补丁校验，列在属性 schema 中，会被持久化，并由模板、剪贴板与同步载荷携带：`levels` 及 `level_reverse`、`level_show_prices`、`level_show_values`、`level_show_percents`、`level_label_align`，以及（仅回撤、扩展与通道）`level_log_scale`；`gann_fans` 与 `gann_arcs`（江恩方形与固定江恩方形）；`wave_degree`（艾略特波浪）；`screen_x` 与 `screen_y`（锚定文本）；`icon_name` 与 `icon_size`（图标戳，8 到 96 CSS px，默认 24）；`bars_pattern` 及 `bars_pattern_mirror_x`、`bars_pattern_mirror_y` 与 `bars_pattern_mode`；以及 `regression_source_id` 及 `regression_deviations`（0 到 10，默认 2）。`wave_degree` 接受上游的九个级别（从 `subminuette` 到 `grand_supercycle`，默认 `minor`），外加自有线的 `submillennium`、`millennium` 与 `supermillennium`。`bars_pattern_mode` 接受 `bars`、`oc_bars`（每根柱一根开盘-收盘竖线）以及 `line_open`、`line_high`、`line_low` 与 `line_close` 这几种线，并把 `hl_bars` 读作 `bars`。一个上限 `MAX_BARS_PATTERN_BARS` = 512（位于 `drawings.rs`，并在 crate 根再导出）限定柱形态快照的大小。`DrawingKindOptions` 把这些字段投影为 `Levels`、`GannSquare`、`RegressionTrend { source_id, deviations }`、`Elliott`、`AnchoredText`、`IconStamp` 与 `BarsPattern`；族工具则投影为 `Line`、`Channel` 与 `ProjectionAnnotation`。

## 族选项

绘图选项遵循上游的数据模型，自有线的类型化块则与之并存。`Drawing.tool_options: DrawingToolOptions` 为每个自有线族保留一个可选块，由选项 JSON、模板、剪贴板与同步载荷以及持久化放在 `tool_options` 之下携带。补丁采用深度合并（缺失的键保留其值，`null` 重置一个块），作用于一份副本，该副本经过校验并限定大小后，再一次性完成可撤销的安装。族工具读取各自的块（`tool_options.line`、`tool_options.channel`、`tool_options.projection_annotation`）。与上游扁平字段重叠的自有线键由同一个辅助函数 `drawing_contract::take_legacy_flat_options(kind, json, absent_block_is_default)` 从 JSON 中取出：它按键是否存在进行映射（只发送块中部分键的补丁，恰好只映射它发送的那些键），并让显式给出的扁平键优先；`apply_patch`（因而包括模板与粘贴）和持久化都使用它。映射如下：斐波那契块的 `reverse`、`log_scale`、`show_prices`、`show_levels`、`levels_as_percent` 与 `label_h_align` 分别变为 `level_reverse`、`level_log_scale`、`level_show_prices`、`level_show_values`、`level_show_percents` 与 `level_label_align`，其中 `reverse` 在扩展、通道与时间区上按原样映射，在回撤与速度扇形上取反（自有线的 0 层级位于它们的第二个锚点上，上游的则位于第一个锚点上），而在螺旋线（上游没有的逆时针旋转）以及从不读取它的工具上完全不映射；江恩块的 `reverse`、`angles` 与 `arcs` 变为 `level_reverse`、`gann_fans` 与 `gann_arcs`；形态块的 `degree` 变为 `wave_degree`；标注块中柱形态的 `bars_mode`、`mirrored`、`flipped` 与 `bars` 变为 `bars_pattern_mode`、`bars_pattern_mirror_x`、`bars_pattern_mirror_y` 与 `bars_pattern`，其 `icon` 与 `icon_size` 变为 `icon_name` 与 `icon_size`（钳制到 96）；通道块的回归偏差变为 `regression_deviations`。没有扁平对应项的自有线键仍会被存储与持久化，但对由上游渲染的种类不起作用：它们不列在 schema 中，也不会被渲染。携带 `tool_options` 的模板转而替换绘图的族样式（`DrawingToolOptions::replacement_patch` 针对绘图的 `template_style`），因此应用它时，也会把它保留为默认值的那些族选项重置，而所捕获的数据（柱形态的快照）不属于样式，因此保留。持久化仅在每个可选样式字段与该种类自身的默认值不同时才写入，因此恢复后的射线保留 `extend_right`，而用户清除过它的射线仍保持清除；`labels` 与 `levels` 在与 `Drawing::new(kind)` 不同时写入，被清空的列表写为 `[]`，而上游的 `gann_fans`、`gann_arcs` 与 `level_*` 键对其种类总是写入。

## 旧版工具名与文档迁移

旧版工具名仅用于输入。自有线在采用上游目录之前写出的 28 个拼写，经由变体上的 `#[serde(alias)]` 以及 `drawings.rs` 中的 `pub(crate)` 表 `LEGACY_DRAWING_KIND_NAMES`（`DrawingKind::from_name` 会回退到该表）解析为代表同一工具的上游种类：`date_and_price_range` → `date_price_range`，十个 `fib_*` 与 `trend_based_fib_*` 名称 → `fibonacci_*` 种类，`xabcd_pattern`、`cypher_pattern`、`abcd_pattern`、`head_and_shoulders`、`triangle_pattern` 与 `three_drives_pattern` → `pattern_*` 种类，五个 `elliott_*_wave` 与 `elliott_*_combo` 名称 → `elliott_*` 种类，`arrow_mark_*` → `arrow_marker_*`，`icon` → `icon_stamp`，以及 `flat_top_bottom` → `flat_top_channel`（持久化会依据所存储的锚点选择平顶通道或平底通道）。每一项输出都写出目录名称（`DrawingKind::name`）。浏览器包以 `drawing_kind_alias` 与 `DRAWING_KIND_ALIASES` 镜像该表；其 wire 表 `DRAWING_KIND_TO_U8` 只包含规范行，因此 `impl.ts` 由它派生出的反向映射（`DRAWING_KIND_FROM_U8`）保持规范。

自有线写出的文档仍可加载。每次导出都会在状态层级写出 `drawing_catalog: 2`（V1、V2 与 V3）。不带该标记的文档，只有在携带自有线写过而任何上游固定修订都未写过的内容时，才被视为自有线写出的文档：旧版或自有线的种类名称、锚点 `time`、带 `tool_options` 的样式、`drawing_price_basis`、不带 `screen_x` 的锚定文本，或缺少上游导出器会为其种类的每个绘图写出的某个字段的 B8 绘图（`lacks_upstream_b8_fields`：带层级工具的 `level_*` 标志、回归的 `regression_deviations`、图标戳的 `icon_size`、柱形态的 `bars_pattern`、艾略特波浪的 `wave_degree`；B8 之前的上游固定修订没有这些种类中的任何一个）。最后这一迹象用于识别写在非时间序列轴上的自有线文档，在这种轴上自有线不写锚点 `time`。来自上游固定修订的文档既没有该标记，也没有此类迹象，因而不经转换地加载；剩余的歧义是不带标记、其 B8 绘图全都不携带这些字段（圆弧、曲线、旋转矩形、正弦线以及其他不带层级的形状）的序列轴文档，它会不经转换地加载。恢复是原子的，因此在检查锚点数之前，一次以上游从不存储的锚点数为键的无条件预处理会转换自有线的锚点契约：`disjoint_channel` 3 → 4（镜像斜率的第二条线变为显式锚点），`gann_square_fixed` 1 → 2（由 `size_bars`、`scale_ratio` 与 `reverse` 得出的对角），`projection` 3 → 顶点与价格点，`price_note` 2 → 其价格点，`signpost` 1 → 底部的两个锚点，`bars_pattern` 2 → 3（由框与复制的柱得出目标），`pattern_triangle` 4 → 5（E 延长 A–C 边），以及 `pattern_three_drives` 7 → 其前六个。只有自有线写出的文档才会同时转换那些锚点数不变但含义改变的种类：`arc` 交换其第二与第三个锚点，`curve` 与 `double_curve` 把其曲线上的点转为贝塞尔控制点，`rotated_rectangle` 把其对称的短边中点转为一条边加一个深度手柄，`fibonacci_speed_arcs` 交换其锚点（上游以第二个锚点为弧的中心），`fibonacci_circles` 把其第二个锚点移到两锚点的中点（上游以第二个锚点为圆心、以完整距离为半径），`sine_line` 把其第一个锚点移到中点（上游的锚点是一个零点与下一个极值点，自有线的则是两个相反的极值点）。位于某个已存储锚点位置上的转换后锚点会保留该锚点的时间标识。自有线的锚定文本把其窗格比例锚点作为 `screen_x`/`screen_y`。只有自有线写出的文档会在 `Drawing::new` 之后、其样式应用之前经过 `kinds::apply_legacy_fork_defaults`，因为自有线省略了每个等于其自身默认值的值（十一个斐波那契层级、中性的虚线颜色、信息线的五项统计、艾略特级别 `intermediate` 等等），并且这种文档的自有线选项键会经 `take_legacy_flat_options` 映射，缺失的键以自有线块的默认值代替。这些有损转换是确定的并有记录：三驱形态丢弃其最后一段，投影丢失其扇区半径，价格便签丢失其标签偏移，路标的杆从零高度开始，柱形态丢失其按框拟合的缩放，`flat_top_bottom` 丢失其交叉处的拆分，未设置 `scale_ratio` 时固定江恩方形的第二个锚点可能远离该方形，旋转矩形在屏幕上可能沿其轴线滑动，速度弧朝其另一个锚点张开而不是向上或向下，正弦波只从其零点起绘制，转换得到的中点位于算术价格中点上（自有线的圆心是像素中点，在对数比例尺上二者不同），反向的螺旋线丢失其逆时针旋转，回归线不对称或单侧的偏差会以启用的较宽一侧合并为一条对称区带。

剪贴板与绘图同步载荷的条目会应用以锚点数为键的锚点转换，但不应用锚点数不变的转换，因为后者需要文档的出处（`persistence::migrate_fork_payload_item`，由 `paste_drawing_items` 与 `apply_drawing_sync_payload_json` 在检查锚点数之前调用）：不带快照的自有线柱形态取其 `tool_options` 中的柱，而选项中不带 `screen_x` 的锚定文本（每个上游载荷都会写出它）把其窗格比例锚点作为屏幕位置。

## 系列数据读取方

回归趋势线、预测与柱形态通过同一个解析器 `ChartEngine::drawing_source_series` 读取系列数据，帧、失效与捕获共用它。在绘图的窗格与价格比例尺上存活的显式 `regression_source_id` 优先，而在那里不存活的显式 id 不给出任何源，因此绘图会等待，而不是度量另一个系列。否则，源为该窗格与比例尺上按创建顺序的第一个存活的普通系列（最小的存活 `SeriesId`：标识单调，存储槽位会被复用；指标输出与自定义系列绝不符合条件，足迹图系列与 feature 系列则通过其 OHLC 投影符合条件，绘制顺序与可见性都不会改变它）。三者都通过 `ChartEngine::drawing_source_window` 读取源的规范行：即回放时钟已揭示的行，对 as-of 源则把每个规范行只读取一次，取其时间处或其后的第一个坐标轴点。

`regression_points` 取 `kinds::channels::regression_stats` 对收盘价的记忆化拟合结果，即对位置位于两锚点之间的柱（上游的窗口：从较早锚点的 `ceil` 到较晚锚点的 `floor`）做一次无内存分配的平移求和遍历，并把区带向两侧各展开 `regression_deviations` 个总体残差偏差。窗口内位于不同柱上的有限收盘价少于两个时，不存在拟合（上游的规则），主体解析为 `DrawingBodyGeometry::RegressionWindow`：两锚点之间的虚线段，像线一样绘制与命中，因此位于未来区域、数据加载之前或位于已回退的回放时钟之后的回归线仍保持可见且可选中（自有线的占位方式）。`RegressionMemo`（`DrawingChartSettings::regression_memo`）保存每个绘图最近一次的拟合结果，以系列、合并点的位置（`time_index_generation`；as-of 源还会以每个时间点为键）、回放时钟、柱范围与源作为键，并附带它所读取的源数据代次，以及除最后一行外所有已拟合行的和。引擎会把它路由给指标的每一次系列数据变更（`update_indicators_after_change`）连同其首个变更行一并报告给该备忘缓存，因此，若某次变更使拟合最后一行之前的每一行都保持不变（对最新柱的实时替换，或追加柱），则只需用变更的行扩展该拟合，按顺序加入这些行，使结果与完整遍历逐位相同；发生在更早行的变更、未报告的变更、已移动的点或 as-of 源，则需要承担一次与锚定范围成正比的遍历。平移、缩放与指针命中测试复用该拟合，release 构建的 `perf_gate` 中的 Target N 会在 1,000,000 行的源上，对带有五条回归线的实时 tick 加帧进行计时（见[指标与绘图性能目标](../../development/performance.md#指标与绘图性能目标)）。当该备忘缓存的条目数超出绘图数量 16 项时，已删除绘图的拟合结果会被丢弃。

`forecast_result` 即 `kinds::projection_annotations::forecast_status`：当入场柱之后的某根柱至迟在目标柱处触及目标（上升目标看最高价，下降目标看最低价）时，即视为到达目标，因此入场柱本身从不计入（上游的读取方从入场柱开始读取）；当目标柱之后存在有成交的柱时，预测即告过期（上游在数据到达目标柱时即令其过期）（空白数据行，例如未来的交易时段槽位，不算柱）；并集源通过其 LOD 极值与最新柱查询（对范围为对数复杂度）作答，而 as-of 源没有规范行金字塔，它会在每次数据、坐标轴、时钟或锚点变化时对窗口内的行扫描一次，并按绘图记忆化（`forecast_memo`，与回归备忘缓存一样有界）。`capture_bars_pattern` 保留上游的快照语义（两个源锚点之间至多 512 个有限行，每行位于其相对第一行的坐标轴偏移处，因此空隙得以保留），并通过同一个窗口读取。`invalidate_frame_series` 仅在发生变化的系列是某条回归趋势线或某个预测的源时，才重建保留的绘图层（每次数据变更会扫描一遍绘图列表；结构性的源变更会使整个场景失效），随后为绑定到该系列的分布绘图调用上游的 `invalidate_profile_drawings_using_series`。回归线的坐标键还携带其源的数据代次，因此绘图几何缓存会跟随源。

## 锚定文本与图标戳

锚定文本采用上游的屏幕模型：`screen_x` 与 `screen_y` 是窗格比例，决定它绘制在何处。`ChartEngine::anchored_text_screen_position` 从一个锚点推导它们（该锚点经绘图的窗格与价格比例尺得到的媒体 px，换算为钳制在窗格内的窗格比例），推导发生在绘图被添加时、放置提交时、其锚点被设置时，以及用于放置预览时，因此预览绘制在点击将落下的位置，而不是默认的 0.5/0.5；`drawing_px` 与运行时缓存读取该屏幕位置。自有线的窗格锚定（经 `drawing_anchor_px` 转换的窗格比例锚点、`pane_anchored` 族钩子，以及它在重新定基、磁吸、粘贴与锚点时间代码中的分支）已退役；`drawing_point_px` 作为 `drawing_to_px_for` 之上的便捷函数保留。

图标戳使用上游的图表本地注册表（`DrawingIconRegistry`，受 `MAX_DRAWING_ICONS`、`MAX_DRAWING_ICON_SIZE` 与 `MAX_DRAWING_ICON_NAME_BYTES` 限制），由 `set_drawing_icon` 填入、由 `remove_drawing_icon` 移除。当图标戳的 `icon_name` 下没有注册图像，且该名称是九个内置 `DrawingIcon` 字形之一（`star`、`heart`、`check`、`cross`、`circle`、`square`、`diamond`、`triangle_up`、`triangle_down`）时，帧会绘制该矢量字形，它由 `kinds::projection_annotations::built_in_icon_parts` 构建，并经共享的[部件转换](#部件词汇与转换)转换，因此没有图标资源的文档与宿主仍能正常渲染；其他任何未注册的名称保留上游的彩色占位 `Rect`。

## 各族语义

各族的语义在下面各自的块中说明。

### 线条

<!-- B8: lines — begin -->
线条族（`kinds/lines.rs`）提供转译自 KLineChart overlay 的自有线工具 `horizontal_segment`（240）、`vertical_ray`（241）、`vertical_segment`（242）与 `price_line`（243）。三个线段工具共用同一几何：前两个锚点所成的线，在第一个锚点之外按 `extend_left` 延长，在第二个锚点之外按 `extend_right` 延长，沿各自方向裁剪到窗格，未延长的端点带有端帽。它们带有目录项 `anchor_link`（`DrawingToolSpec::anchor_link`）：每个锚点共享第一个锚点的价格（`SamePrice`）或柱（`SameLogical`），该值取自最后放置或拖动的锚点。

该联动在点列表进入模型处（`Drawing::normalize_points`：构造、导入、程序化锚点以及提交一次放置）以及拖动或预览锚点期间生效，因此被锁定的工具不会离开其轴线；Shift 对它们没有可拉直之处。垂直射线默认使用 `extend_right`，使其穿过第二个锚点延伸到窗格边缘。可见的 `labels` 渲染为一个统计框，其位置由 `tool_options.line.stats_position` 决定。`price_line`（一个锚点）是一条从锚点到窗格右边缘的清晰 `hline`，外加一个位于其起点上方、显示锚点价格的部件标签；它采用水平线的 `axis_price_label` 标签，并按射线方式命中。
<!-- B8: lines — end -->

### 通道

<!-- B8: channels — begin -->
通道族（`kinds/channels.rs`）提供自有线的 `price_channel`（244），即 KLineChart 的价格通道：穿过前两个锚点的基准线为中心线，其平行线穿过第三个锚点（在 px 上垂直平移，因此在每种比例尺模式下都平行，并跨越相同的柱），该平行线的镜像位于另一侧。它默认向两端延伸且不填充，填充覆盖整个区带，用 `shape::clip_polygon_to_rect` 裁剪到窗格，且仅在绘图被选中时才是主体目标。它的价格范围为 `Full`。该族的 `handles` 钩子把第三个锚点的手柄移到平行线的中点上，因此手柄绘制（包括放置预览）、手柄命中测试与键盘微调都在那里读取它，而它仍按指针增量驱动自己的锚点。Shift 会把被拖动的基准线端点相对另一端拉直，与趋势线相同。设置 `partial_preview` 后，放置时在放置第二个锚点期间预览基准线，之后预览穿过指针的整个通道。该模块还拥有上文所述的回归读取方（`regression_stats`、`RegressionMemo`；见[系列数据读取方](#系列数据读取方)）；可选的 `tool_options.channel` 块仅存储已设置的字段。
<!-- B8: channels — end -->

### 投影与标注

<!-- B8: projection_annotations — begin -->
投影与标注族（`kinds/projection_annotations.rs`）提供测量工具 `price_range`（13）、`date_range`（14）与 `date_price_range`（15）——这三个测量工具见[测量工具与瞬态测量](drawings.md#测量工具与瞬态测量)——以及自有线的 `simple_tag`（245）与 `simple_annotation`（246），即 KLineChart 的简单标签与简单标注。它设置 `owns_text` 与 `owns_labels`。简单标签是一条位于其锚点价格处、横贯窗格的虚线，带有水平线的坐标轴标签；当它带有文本时，该标签显示其 `text` 而非价格（`axis_tag_text`），因此其文本从不绘制在图表上，也不可编辑。简单标注是一根从其锚点升起的虚线杆，杆顶是一个小头部，其上方是一个带框文本标签（`DrawingParts::text_label`，因此宿主的内联编辑器可以就地编辑它）；放置它会打开编辑器（`DrawingToolSpec::requests_text_editor`），剔除余量把空文本计为一行，即其编辑器保留的插入符行。它的选项位于 `tool_options.projection_annotation` 中。该模块还拥有预测读取方（`forecast_status`、`ForecastMemo`）、内置图标字形（`DrawingIcon`、`built_in_icon_parts`：填充的单一区域，即凸多边形，或用于星形轮廓的围绕核的三角扇，因此绘制与命中测试覆盖完全相同的区域），以及旧版映射所读取的柱形态模式与图标类型。
<!-- B8: projection_annotations — end -->

## Wire id 与注册表

Wire id `0..=84` 原样沿用上游的表，上游从 85 起连续分配新种类；自有线工具占用 `u8` 空间的顶部 `240..=246`，因此上游的增长绝不会与之冲突，两者之间的空间尚未分配。别名没有 id。id 只穿越进程内的 JS/WASM 边界；文档、模板、剪贴板与同步载荷携带的是名称，因此重新编号不会破坏任何已存储的数据。一项目录标识测试断言：每个规格都具有唯一的 wire id 与名称；id `0..=84` 遵循上游的顺序，自有线工具位于 `240..=246`；`family` 恰好只为十个族工具设置；serde 名称与目录名称一致，并能经 `from_u8` 与 `from_name` 往返；每个旧版名称都解析为其种类，且自身不是目录名称。

单一列表的注册表为每个仍保留的族（lines、channels、projection_annotations）携带一个 `// B8: <family> — begin/end` 块，因此对某个族的并行工作可以以累加方式合并：

| 文件 | 块 |
| --- | --- |
| `drawings/kinds/mod.rs` | `mod` 声明；族专属 `DrawingFamily` 钩子及其在 `DrawingFamily::new` 中的默认值 |
| `drawing_contract.rs` | `Line`、`Channel` 与 `ProjectionAnnotation` 三个 `DrawingKindOptions` 变体；`DrawingToolOptions` 字段与 `validate` 检查 |
| `lib.rs` | 族选项类型的公共再导出 |
| [`docs/architecture/engine/drawing-families.md`](drawing-families.md) | 族语义段落（见上文） |

## 添加绘图族

自有线族工具的实现步骤（上游目录中的工具则改走上游的路径：`tools.rs` 中的规格、`geometry.rs` 中的主体、一个帧分支与一个命中测试）：

1. 在自有线块之后添加 `DrawingKind` 变体（serde 名称即规格名称），并在 `kinds/<family>.rs` 中添加其规格，使用 255 以下下一个空闲的 wire id 与 `family: Some(&FAMILY)`。在 `DRAWING_TOOL_SPECS` 中列出该规格及其 `spec()` 分支，并把该名称加入持久化用于识别自有线写出的文档的自有线名称中。
2. 只通过 `DrawingParts` 并使用共享辅助函数来解析几何；当纯几何具有通用性时，将其添加到 `aeris_charts_render::shape`。绝不在帧、命中测试器、执行器、WASM 或宿主中发出 `Prim`，也不得在这些位置按种类分支。用 `PartContext::point_px` 映射派生点，并按 `PartContext.scale` 比例确定描边、字形和间隙的尺寸。派生（非锚点）手柄的位置经由共享的 `handles` 钩子确定（需要自己拖动的手柄须先重新加入 `drag` 钩子；见上文的[延期说明](#目录与共享钩子)）。将绘图自身的 `text` 以 `DrawingParts::text_label` 绘制，基于 `PartContext::text_lines`，这使其无需任何宿主或 WASM 代码即可就地编辑。
3. 将族选项放入该族带 serde 默认值的块中，对列表、字符串或数字添加 `validate` 检查，并添加 `DrawingKindOptions` 投影、新类型的 `lib.rs` 再导出，以及名为 `tool_options.<block>.<field>` 的 schema 描述符。
4. 添加 TypeScript 联合类型成员与 wire id、种类选项与选项类型、演示工具栏按钮，以及 `docs/api/drawings.md` 条目；`impl.ts` 会派生其反向 wire 映射。`packages/charts/api/public-api-v1.json` 是一个生成的哈希；应重新生成它（`bun run update:api`），而不是合并它。
5. 测试：目录标识测试；族引擎测试，覆盖默认值、已激活工具的放置、DPR 为 1 和 2 以及位图比例互不相同的小数 DPR 下的帧部件、命中测试（包括在超过 20 个绘图时将索引化结果与暴力遍历对照）、带拉直与磁吸的锚点拖动与主体拖动、键盘手柄数量与微调、跨周期切换的时间标识、schema 与种类选项、原子且可撤销的选项补丁、省略默认值的持久化往返、剪贴板与同步，以及绘图自身的文本恰好只绘制一次；以及一个 Playwright 规格，覆盖已激活工具的放置、悬停与命中、选项、持久化、剪贴板与同步、工具栏，以及 WebGPU 与 Canvas2D 的一致性。`drawings/tests.rs` 中覆盖整个目录的循环测试（周期切换、索引化命中与暴力遍历对照、无数据时的退化锚点、极端缩放下与对数比例尺上的有限几何，以及剪贴板与同步往返）涵盖每个种类，因此新工具只要位于目录中就会加入这些测试。

`drawing_perf` 原生示例在其 `families` 组合中测量 B8 工具，这些工具按 wire id 从引擎目录读取，并按其目录锚点数放置每个工具，因此添加工具绝不需要编辑它。
