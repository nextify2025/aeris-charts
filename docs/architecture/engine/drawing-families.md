# 绘图族与扩展流程

[文档导航](../../README.md) · [架构总览](../../Architecture.md)

各族共享[绘图状态与历史](drawings.md)、[输入控制器](input.md)和[文本会话](drawing-text.md)。B8 是其在扩展计划中的批次名称，不是另一套运行时。

- [目录与共享钩子](#目录与共享钩子)
- [部件词汇与转换](#部件词汇与转换)
- [族选项](#族选项)
- [各族语义](#各族语义)
- [Wire id 与注册表](#wire-id-与注册表)
- [添加绘图族](#添加绘图族)

## 目录与共享钩子

B8 工具归属于绘图族。核心工具（趋势线、水平线与垂直线、水平射线、矩形、文本、画笔、路径、Long/Short Position 以及分布绘图）在其目录规格中保持 `family: None`，并通过 `drawings/geometry.rs` 解析，而不是通过族部件词汇；它们的虚线与点线描边通过 `push_styled_stroke` 转换为实线虚线段，详见[绘图几何](drawings.md#派生运行时与几何)。每个族拥有一个模块 `drawings/kinds/<family>.rs`，其中包含该族的工具规格、一份被每个规格引用的 `static FAMILY: DrawingFamily` 钩子表、其类型化选项块及其测试（`kinds/<family>/tests.rs`）。钩子表是封闭的编译期表，而不是插件注册表。`DrawingFamily::new` 接收两个必需的钩子 `build_parts` 与 `kind_options`，并让每个可选钩子都从一个中性默认值开始，族在其 `static` 初始化器中通过赋值覆盖它：`apply_defaults`（由 `Drawing::new` 应用的种类默认值，因此创建、模板、恢复、粘贴与 schema 默认值保持一致）、`decoration_extent`（为锚点之外的框与标签提供的保守 CSS-px 剔除余量）、`extend_schema`（追加在通用描述符之后的 `tool_options.*` 描述符，其默认值引擎已取自该种类的模板绘图），以及 `owns_labels`（该族自行渲染通用的 `labels`）。

之后新增的钩子会在 `DrawingFamily::new` 中获得其默认值，因此其他族无需改动即可编译；被多个族设置的钩子，或基础层为每个族新增的钩子，位于族块之外。这些共享钩子是 `paint_bounds`、`reads_series_data`、`partial_preview`、`handles`、`drag` 与 `close_placement`（基础层钩子；目前只有形状折线设置它）。`paint_bounds` 是唯一的族剔除钩子：它是绘图所绘制的除文本之外的一切内容的保守媒体像素包围盒，每当其坐标键变化时，依据其锚点的媒体像素重新计算。范围由屏幕推导的工具（叉形线的叉齿与层级、经过边缘锚点的圆、固定尺寸的正方形）声明 `Full` 逻辑与价格范围，因此没有任何语义包围盒会剔除它；当其边界在两个维度上都保持无界时，该包围盒会取代整个窗格，作为其屏幕剔除与命中候选包围盒（`None` 或带延伸的绘图则保留窗格）。`reads_series_data` 标记其几何会读取系列数据的绘图（回归的拟合、预测的结果）：每个此类绘图都针对 `ChartEngine::drawing_source_series` 进行度量，即其窗格与价格比例尺上按创建顺序的第一个存活的普通系列（最小的存活 `SeriesId`；标识单调，存储槽位会被复用）（指标输出与自定义系列绝不符合条件，足迹图系列与 feature 系列则通过其 OHLC 投影符合条件，绘制顺序与可见性都不会改变它），并且 `invalidate_frame_series` 仅在发生变化的系列是此类绘图的源时，才重建保留的绘图层（每次数据变更会扫描一遍绘图列表；结构性的源变更会使整个场景失效）。

这些读取方在源的规范行上工作：as-of 源的绘制点会重复并跳过规范行，因此回归、预测与柱形态会对每个规范行只读取一次，取其时间处或其后的第一个坐标轴点（参见 `ChartEngine::drawing_source_window`）。`partial_preview` 让放置预览从第二个锚点起即可解析部件。`handles` 与 `drag` 是派生手柄的基础：`handles` 编辑 `drawings/handles.rs` 依据规格的手柄模式以媒体像素构建的手柄集合（把某个手柄移到派生几何上、删除某个手柄，或追加驱动 `DrawingDragPart::Handle(index)` 的手柄），而选中手柄的绘制、放置预览（仅限已放置锚点的手柄）、手柄命中测试、键盘手柄循环与拖动起始，全都读取这同一个集合。随后每个拖动采样（无论是指针拖动还是键盘微调）都会运行通用的部件拖动（锚点经时间吸附、磁吸与拉直后重新锚定；本体平移；派生手柄的基线媒体像素沿移动轴按增量移动，并像锚点一样进行时间吸附与磁吸），并把结果作为 `HandleDrag` 采样交给 `drag`（基线锚点与像素、以锚点形式及像素形式表示的被拖动点、Shift，以及键盘微调的步长，因此对其目标做量化的钩子，例如固定江恩正方形的整根柱，每次按键至少移动一个单位）。

微调仅在记录了可撤销的变更时才报告成功。钩子仅依据该基线改写锚点，并可返回替换用的工具选项，拖动会话的历史快照会在取消时恢复这些选项，并在同一个撤销步骤中记录它们；由数据驱动的基线重设也会把派生手柄的基线重新定基到其当前位置。`close_placement` 让多次点击的工具可在其第一个顶点处闭合：已放置至少三个顶点时，在第一个顶点的精确锚点命中半径内点击会调用该钩子（折线会设置 `closed`）并在不添加顶点的情况下提交，并且悬停在那里时预览会吸附到该顶点；Enter、双击与 Escape 仍保持以开放形式完成与取消的行为。钩子在帧构建与命中测试期间、绘图运行时缓存处于借用状态时运行，因此它们只读取引擎，绝不调用候选查询或缓存的锚点几何访问器。规格字段取代共享代码中逐种类的检查：`text_layout`（`Box`，或 `Segment`：标签跟随前两个锚点、随之旋转并取其描边颜色，居中标签会拆分描边）、`axis_price_label`（水平线的坐标轴标签）以及 `axis_tag_text`（当绘图带有文本时，该标签显示该绘图的 `text` 而非价格，且该文本不会绘制在图表上，与简单标签的做法一致）。


## 部件词汇与转换

一个绘图族会在调用方的空间（渲染时为位图 px，命中测试时为媒体 px）中，把一个绘图解析为 `drawings/parts.rs` 中共享的部件词汇：抗锯齿描边、清晰的整像素水平线与垂直线、成对链之间的带状填充（凸多边形通过 `fill_convex`，任意多边形按非零规则通过 `fill_polygon`）、圆盘、每个像素只绘制一次的宽描边（`Tube`，转换为一次区带填充），以及由同一个 `PartLabel::layout` 排版的带框文本块。派生的逻辑/价格点（层级线、时间区、数据驱动的点）通过 `PartContext::point_px` 映射到该空间，该函数应用帧各自独立的水平与垂直位图比例；帧构建会以 debug 断言确认它能复现每一个锚点。帧构建在 `frame/drawings.rs` 中把部件转换为现有的 `Prim`（`build_family_prims`；每一段描边都经过 `push_clipped_stroke`，它用 `shape::clip_polyline_to_rect` 把描边裁剪到按描边延伸量外扩后的窗格范围内，因此无论几何延伸多远，工作量与坐标都保持有界，并且和引擎拥有的所有路径描边一样，通过 `push_line_stroke` 把虚线或点线描边段拆分为实线的短划段，因为 WebGPU 细分器没有虚线概念；被裁剪的部件保留未裁剪描边段的虚线相位，因此平移时虚线绝不会偏移），而精确命中测试检测的是同样的部件（`DrawingParts::hit`，其中虚线描边仍是一个连续的主体），所以各族工具不能分叉执行器行为，绘制出的形状与可交互的形状也不会漂移。

工具放置期间，一旦每个锚点都已放置或处于预览中（设置 `partial_preview` 时，从第二个锚点起），帧的待定路径就会构建该工具自身的部件；在此之前，具有三个或更多锚点的工具会把已放置的锚点与指针连成一条以绘图描边绘制的引导折线，该折线经 `push_clipped_stroke` 转换，并在已放置的锚点上显示手柄，因此每次点击都会留下可见的笔迹。共享辅助函数位于各自的所有者处：纯几何（线段延长、中点以及到窗格的裁剪、射线裁剪、平行偏移、弧/椭圆细分与感知裁剪的曲线展平、非零规则多边形带、折线/多边形/带状命中判定，以及 `Rect` 的外扩、求交与包围盒）位于 `aeris_charts_render::shape`；线帽（`capped_segment` 就是两点的 `capped_polyline`，`cap_radius` 决定每个圆盘与箭头的尺寸）、箭头修剪、标签排版、统计框（`PartContext::stats_label`，使用共享的 `STATS_*` 间距、内边距与 alpha，以及 `text_on` 选出的黑色或白色文字）以及填充约定（`PartContext::fills_hit`：区域填充仅在绘图被选中时才是主体目标，与矩形内部相同）位于 `parts.rs`；颜色解析位于契约类型上（`Drawing::stroke_color` 带规范的主色回退，`Drawing::fill_or_wash`，以及 `DrawingLevel::stroke_color`、`zone_fill` 和 `line_style`，其样式名与绘图自身的 `style` 一样经由 `line_style_from_name` 归并）；属性描述符构建器 `drawing_contract::descriptor`；引擎格式化的测量文本（价格经绘图比例尺的格式化器、百分比、tick 数、柱数、时间范围、时长、屏幕角度、距离）以及文本与统计字形大小（`drawing_text_size`、`drawing_stats_size`）位于 `drawings/stats.rs`；绘制、手柄命中测试、键盘循环切换与拖动共用的唯一可编辑手柄集合，连同拖动所用的 `HandleDrag` 采样与 `drawing_anchor_at` 转换族，位于 `drawings/handles.rs`；命中测试与手柄钩子使用的媒体 px 版 `PartContext::media` 位于 `parts.rs`；以及作为契约数据的层级列表（`FIBONACCI_RATIOS`、`FIBONACCI_TIME_ZONES`、`drawing_levels_from_ratios`、`DrawingLevel::price_between`、`DrawingLevel::label`）。

设置了 `extend_left` 或 `extend_right` 的绘图使用无界的语义边界，因此锚点滚出视野时，延长线仍保持可见且可命中；而关闭了延长的射线或延长线，则与任何有限线段一样被剔除。

## 族选项

各族专属选项位于 `Drawing.tool_options: DrawingToolOptions` 中，每个族对应一个可选块，由选项 JSON、模板、剪贴板与同步载荷以及 V1 持久化放在 `tool_options` 之下携带。补丁采用深度合并（缺失的键保留其值，`null` 重置一个块），作用于一份副本，该副本经过校验并限定大小后，再一次性完成可撤销的安装。携带 `tool_options` 的模板转而替换绘图的族样式（`DrawingToolOptions::replacement_patch` 针对绘图的 `template_style`），因此应用它时，也会把它保留为默认值的那些族选项重置，而绘图所捕获的数据（柱形态的副本）不属于样式，因此保留。持久化仅在每个可选样式字段与该类型自身的默认值不同时才写入，因此恢复后的射线保留 `extend_right`，而用户清除过它的射线仍保持清除。

## 各族语义

各族的语义在下面各自的块中说明。

### 线条

<!-- B8: lines — begin -->
线条族（`kinds/lines.rs`，wire id 32..=47）提供 `ray`、`extended_line`、`info_line`、`trend_angle`、`cross_line` 与 `arrow_line`。五个线段工具共用同一几何：前两个锚点所成的线，在第一个锚点之外按 `extend_left` 延长，在第二个锚点之外按 `extend_right` 延长（射线与延长线即取这些默认值），沿各自方向裁剪到窗格，未延长的端点带有端帽。可见的 `labels` 渲染为一个统计框，其位置由 `tool_options.line.stats_position` 决定；信息线默认启用价格变化、百分比变化、柱数、时长与角度。趋势角度线增加一条虚线水平参考线、通向线段的圆弧以及屏幕角度。十字线是贯穿整个跨度的清晰水平线与垂直线，并带有水平线的坐标轴价格标签。`horizontal_segment`、`vertical_ray` 与 `vertical_segment`（wire id 38..=40，转译自 KLineChart 的 overlay）使用相同的线段几何，并带有目录项 `anchor_link`（`DrawingToolSpec::anchor_link`）：每个锚点共享第一个锚点的价格（`SamePrice`）或柱（`SameLogical`），该值取自最后放置或拖动的锚点。

该联动在点列表进入模型处（`Drawing::normalize_points`：构造、导入、程序化锚点以及提交一次放置）以及拖动或预览锚点期间生效，因此被锁定的工具不会离开其轴线；Shift 对它们没有可拉直之处。垂直射线默认使用 `extend_right`，使其穿过第二个锚点延伸到窗格边缘。`price_line`（wire id 41，一个锚点）是一条从锚点到窗格右边缘的清晰 `hline`，外加一个位于其起点上方、显示锚点价格的部件标签；它采用水平线的 `axis_price_label` 标签，并按射线方式命中。
<!-- B8: lines — end -->

### 通道

<!-- B8: channels — begin -->
通道族（`kinds/channels.rs`，wire id 48..=52）提供 `parallel_channel`、`regression_trend`、`flat_top_bottom`、`disjoint_channel` 与 `price_channel`（KLineChart 的价格通道：基准线为中心线，其平行线穿过第三个锚点，该平行线的镜像位于另一侧；默认向两端延伸且不填充，填充会覆盖整个区带）。三锚点通道共用同一种构造：前两个锚点构成基准线，第二条线在穿过第三个锚点的直线上跨越相同的柱（两侧为垂直边）——在 px 上垂直平移（在每种比例尺模式下都平行）、在第三个锚点的价格处保持水平，或取基准斜率的镜像。除平顶/平底通道外，它们的价格范围均为 `Full`，因为第二条线的端点可能离开锚点的价格框。填充在两条线之间沿公共参数跨度展开，在唯一的交叉点处拆分，使每一块都是凸的，并用 `shape::clip_polygon_to_rect` 裁剪到窗格；它们仅在绘图被选中时才是主体目标。该族共享的 `handles` 钩子把锚点手柄移到已绘制的线上（第三个锚点的手柄移到第二条线的中点，回归线的手柄移到其拟合线的两端），因此手柄绘制（包括放置预览）、手柄命中测试与键盘微调都在那里读取它们，而每个手柄仍按指针增量驱动各自的锚点。Shift 会把被拖动的基准线端点相对另一端拉直，与趋势线相同（锚点拖动会拉直任何具有拉直模式的工具的前两个锚点）。设置 `partial_preview` 后，放置时在放置第二个锚点期间预览基准线，之后预览穿过指针的整个通道。

`regression_trend` 在其取整后的锚点柱之间，对其规范行上的源系列（`drawing_source_series`）做拟合，通过一次无内存分配的平移求和遍历完成（最小二乘斜率、样本残差偏差、Pearson R）。`RegressionMemo`（`DrawingChartSettings::regression_memo`）保存每个绘图最近一次的拟合结果，以系列、合并点的位置（`time_index_generation`；as-of 源还会以每个时间点为键）、回放时钟、柱范围与源作为键，并附带它所读取的源数据代次，以及除最后一行外所有已拟合行的和。引擎会把它路由给指标的每一次系列数据变更（`update_indicators_after_change`）连同其首个变更行一并报告给该备忘缓存，因此，若某次变更使拟合最后一行之前的每一行都保持不变（对最新柱的实时替换，或追加柱），则只需用变更的行扩展该拟合，按顺序加入这些行，使结果与完整遍历逐位相同；发生在更早行的变更、未报告的变更、已移动的点或 as-of 源，则需要承担一次与锚定范围成正比的遍历。平移、缩放与指针命中测试复用该拟合，无论图表持有多少条回归线，且 release 构建的 `perf_gate` 中的 Target N 会在 1,000,000 行的源上，对带有五条回归线的实时 tick 加帧进行计时。当该备忘缓存的条目数超出绘图数量 16 项时，已删除绘图的拟合结果会被丢弃。它的锚点只沿时间方向移动，并设置共享的 `reads_series_data` 钩子。可选的 `tool_options.channel` 块仅存储已设置的字段；各工具的默认值在使用时解析。
<!-- B8: channels — end -->

### 斐波那契

<!-- B8: fibonacci — begin -->
斐波那契族（`kinds/fibonacci.rs`，wire id 64..=73，位于 64..=95 范围内）提供 `fib_retracement`、`trend_based_fib_extension`、`fib_channel`、`fib_time_zone`、`trend_based_fib_time`、`fib_speed_resistance_fan`、`fib_speed_resistance_arcs`、`fib_circles`、`fib_spiral` 与 `fib_wedge`。除螺旋线外，每个工具都绘制通用的层级列表（`Drawing::levels`，默认值由 `apply_defaults` 设置）：可见层级按值排序，相邻层级之间的区带在 `fill_enabled` 开关下采用较高层级的填充，标签采用层级颜色。绘图自身的描边是辅助线（趋势线、扇形网格、楔形边缘或螺旋线）。价格层级（回撤、扩展、通道）在价格空间中计算，设置 `tool_options.fibonacci.log_scale` 时在对数空间中计算，并通过 `PartContext::point_px` 映射，因此在每种比例尺模式下都落在精确的价格上；时间层级对锚点的 px 做插值，因为时间轴相对逻辑位置是仿射的；扇形、弧、圆、螺旋线与楔形是由锚点 px 得出的屏幕空间几何。区带填充仅在绘图被选中时才是主体目标。

该族新增 `bounds` 钩子（在 `DrawingFamily::new` 中属于该族的块里取默认值），它返回绘图完整的语义范围（`FamilyBounds`：逻辑与价格范围，无界时为 `None`），并取代 `DrawingBounds::for_drawing` 中由锚点推导出的剔除边界，因此锚点滚出视野时，锚点之外的层级仍保持可见且可命中；屏幕空间工具保持无界的规格范围，并跳过窗格之外的曲线。弧、圆与楔形把共享的 `paint_bounds` 设为锚点加上以其中心为中心、触及最大可见层级半径的正方形（至少为单位大小），其 `decoration_extent` 再按层级标签对它外扩，因此远离窗格的环形工具与任何有限绘图一样被剔除并跳过命中测试；扇形的射线与螺旋线延伸到窗格边缘，保持为整个窗格。它设置共享的 `partial_preview`，因此三锚点工具在第二次点击之前就会显示其第一段。层级列表仅在与该类型的默认值不同时才持久化，因此用户清空的列表保持清空。

无论层级取值如何，每个输出的坐标都保持在窗格的可达范围内：扇形射线止于窗格边缘，不与窗格相交的通道线会被跳过，超出最远窗格点的圆环半径会在该距离处闭合区带与楔形边缘。扇形区与通道区带用半平面裁剪窗格多边形（`aeris_charts_render::shape::clip_to_half_plane`），因此延长的通道区带会覆盖其线条离开窗格所经过的窗格角。标签按其自身的框剔除（每个字符按一个字形大小计），而不是按其层级线剔除，因此即使标签所属的线刚好位于窗格之外，该标签仍会绘制。螺旋线从亚像素半径起，每四分之一圈增长 φ，直到其半径超过最远窗格点为止，至多 128 个四分之一圈。弧、圆、楔形弧与螺旋圈会跳过无法到达窗格的部分，并且在其圆心位于窗格之外时，只对窗格所对的角度窗口做细分，因此远在窗格之外的曲线，在受上限约束的线段数下仍保持在曲线容差之内；同一工具的弧共用一张单位角度表，该表也用于配对它的区带链。带窗口的虚线或点线曲线从窗口之前最后一个虚线周期边界开始（弧长从曲线自身的起点算起，位于窗格之外），完整的圆则在整圆本身重启的位置重启其图案，因此窗格滚动时虚线保持不动。
<!-- B8: fibonacci — end -->

### 叉形线与江恩

<!-- B8: pitchforks_gann — begin -->
叉形线与江恩族（`kinds/pitchforks_gann.rs`，wire id 96..=127）提供 `andrews_pitchfork`、`schiff_pitchfork`、`modified_schiff_pitchfork`、`inside_pitchfork`、`pitchfan`、`gann_box`、`gann_square`、`gann_square_fixed` 与 `gann_fan`。叉形线根据其锚点 A、B、C 解析出一套几何框架：中线枢轴（A；Schiff：取 A 的时间与 A、B 价格的中点；修正 Schiff 与内部叉形线：A 与 B 的中点）、基线中心（B 与 C 的中点；内部叉形线：C）以及半手柄（指向 C；内部叉形线：回指 B）。绘图 `levels` 的层级 `v` 是一对平行于中线、穿过 `center ± v · half` 的叉齿，因此层级 1 穿过手柄端点。未延长的线越过基线延伸一个中线的长度；`extend_left`/`extend_right` 把每条线都延伸到窗格边缘，而延长的区域填充通过 `shape::clip_polygon_to_rect` 裁剪到窗格。

移位枢轴变体增加一条虚线 A–B 引导线，而 pitchfan 把各层级绘制为从 A 出发、穿过 Andrews 基线的射线。江恩框用价格 `levels` 与 `tool_options.gann.time_levels` 划分其角点所围的框，带有区域填充、四边的比例标签，以及从枢轴角点出发的可选 `angles`。与矩形内部一样，该族的每一种区域填充（叉形线条带、扇区、江恩框区域、方形弧）仅在其绘图被选中时才是拖动目标。江恩方形绘制 `levels` 网格、`angles` 扇形，以及围绕其枢轴角点的四分之一椭圆 `arcs`，外加一个由引擎格式化的价格范围、柱数与每柱价格框。固定方形由一个锚点、`size_bars` 与可选的 `scale_ratio` 构成（没有它时，它在屏幕上是正方形）。江恩扇形的 `levels` 是 1×1 斜率的倍数，该斜率穿过第二个锚点，或每柱上升 `scale_ratio` 个价格。它的线默认是射线，标签为 `8x1` 到 `1x8`。该族的每个工具都会越过其锚点延伸一个与视口相关的量，因此各规格声明 `Full` 范围，而该族的 `paint_bounds` 在媒体 px 中解析相同的角点与线端（延长的绘图保持整个窗格）。

清晰的水平与垂直线按 `push_clipped_stroke` 的虚线相位规则钳制到窗格（`line::crisp_span`；执行器从其起始像素起逐像素绘制其虚线），窗格之外的标签框不输出任何内容，因此极端的层级、大小或缩放绝不会使帧工作量随几何长度增长，也不会留下非有限坐标。通过共享的 `handles` 与 `drag` 钩子，叉形线与 pitchfan 在 B 与 C 的基线中点上增加第四个手柄，它按该中点（经磁吸）的移动量同时平移 B 与 C；固定方形则增加其远端角点，用于调整其大小：角点的时间以整柱设置 `size_bars`（至少为 1），角点位于锚点的哪一侧则设置 `reverse`；有 `scale_ratio` 时，角点的价格设置该比例（Shift 保持比例不变），没有时方形在屏幕上保持正方形，其大小由角点到锚点的较大距离决定。指针会把边长取整到最近的柱；键盘步进至少沿按键移动的方向移动一整根柱（没有比例时按移动的那根轴确定大小），因此不足一柱的按键会累积。选项的编辑属于该次拖动的同一个撤销步骤。

扇形与固定方形的 `scale_ratio` 是每柱的价格，因此该族的 `rescale_price_options` 会在枢轴锚点发生价格基准重新缩放时缩放它。
<!-- B8: pitchforks_gann — end -->

### 投影与标注

<!-- B8: projection_annotations — begin -->
投影与标注族（`kinds/projection_annotations.rs`，wire id 128..=159）提供 `forecast`、`bars_pattern`、`price_range`、`date_range`、`date_and_price_range`、`projection`、`anchored_text`、`note`、`price_note`、`callout`、`comment`、`price_label`、`signpost`、`flag_mark`、`arrow_mark_up`/`down`/`left`/`right`、`icon`，以及 KLineChart 的 `simple_tag` 与 `simple_annotation`（wire id 147 与 148）。它的选项位于 `tool_options.projection_annotation` 中（柱形态模式、镜像、翻转与已复制的柱；图标与图标大小）。它向 `DrawingFamily` 添加四个钩子，默认均为中性：`on_create`（在新绘图被存储之前调用，由已激活工具的放置提交调用——该提交始终会进行捕获，因为其选项来自工具模板——以及由 `add_drawing` 调用，粘贴同样使用它，且它会保留其选项中已携带的状态；同步与恢复从不调用它）、`owns_text`（通用文本遍历会跳过该族的工具；它们把通用 `text` 布局在各自的部件中）、`pane_anchored`（按类型）与 `reveals_on_focus`（仅在悬停、选中或编辑时才绘制部分部件的绘图；见下方说明）。

窗格锚定的类型（`anchored_text`）把其锚点存储为窗格比例（`logical` = x / 窗格宽度，`price` = y / 窗格高度）：每一次锚点转换都经过 `ChartEngine::drawing_anchor_px`/`drawing_anchor_from_px`（以及 `drawing_point_px`），因此帧、命中测试、手柄、拖动、微调、创建、统计与 `PartContext::point_px` 保持一致。它的锚点不携带时间（`set_pending_times` 会丢弃它们，`resolve_drawing_anchors` 忽略 `time`，恢复时忽略非时间锚点的锚点时间附带数据），锚点解析与拖动把其比例钳制到 `0..=1`（恢复会拒绝超出该范围的文档），并且它绝不会因数据变更而被重新定基、做价格重新缩放、磁吸，或被粘贴或成组移动所偏移。剔除对它使用完整范围；投影扇区（屏幕 px 的圆）也声明完整范围，该族的 `paint_bounds` 以包含其顶点并触及半径点的正方形为其界，并通过 `decoration_extent` 按其统计框外扩。


`forecast` 从其源系列（`drawing_source_series`，经共享的 `reads_series_data` 钩子跟踪，因此该系列中即使没有移动任何比例尺的 Tick 也会更新它）出发，通过 LOD 极值与最新柱查询（对范围为对数复杂度）来评估其结果：当源柱之后的某根柱至迟在目标柱处到达目标，即为成功；当目标柱之后存在有成交的柱，即为失败（空白数据行，例如未来的交易时段槽位，不算柱）。as-of 源没有规范行金字塔（其 LOD 汇总的是绘制点），因此其结果会在每次数据、坐标轴、时钟或锚点变化时对窗口内的规范行扫描一次，并像回归拟合一样按绘图记忆化。`bars_pattern` 在创建时一次性复制至多 128 根 OHLC 柱（更长的范围会聚合为 128 个桶，每个桶读取自其 LOD 汇总行，因此捕获以及每帧重复它的放置预览对范围保持对数复杂度；as-of 源的桶扫描其规范行，受捕获窗口限制），将其锚点固定在副本的外框上（第一根柱位于最高值，最后一根柱位于最低值），把副本拟合进锚点所围的框（以副本的完整范围为除数，因此小幅拖动锚点会按比例缩放它，价格基准重新缩放则会精确缩放它；flip 使其在框内上下颠倒，mirror 使时间反向），并且每帧只转换窗格内的列。

虚影从不超出锚点所围的框，因此它像任何有限绘图一样被剔除。第一次点击后，投影显示共享的放置引导；第二次点击后，它通过指针预览扇区。具名模板只携带样式：`DrawingToolOptions::template_style` 会丢弃复制的柱，因此应用模板只会重新设置形态的样式而不替换其副本，并且与所有模板一样，它从不携带标识、放置、可见性或文本内容（见[类型化契约与模板](drawings.md#类型化契约与模板)）。填充标记是单一区域：凸多边形、成对链轮廓（箭头标记），或围绕核的三角扇（用于星形轮廓，即星形与心形图标），因此绘制与命中测试覆盖完全相同的区域。持久化会将 `text` 与该种类的默认值比较，因此被清除的默认标签保持为已清除。锚定文本、便签、价格便签、标注框、评论、价格标签、路标和箭头标记的文本框都是共享文本标签（`DrawingParts::text_label`；价格便签与价格标签的文本跟随其价格线），因此宿主的内联编辑器可以就地编辑它们。放置锚定文本、便签、标注框、评论、路标或简单标注会立即打开该编辑器（`DrawingToolSpec::requests_text_editor`，以 `request_text_edit` 上报），因为它们各自都从一段默认文本开始，用户会替换或扩展该文本；价格便签、价格标签和箭头标记起初没有自己的文本，因此不会打开。

该标志位只请求打开编辑器：文本工具的文本焦点边框遵循 `DrawingHandleMode::None`，因此已放置的便签或标注框保留其锚点手柄。旗标、图标、简单标签（其文本即坐标轴标签），以及投影与测量工具，都不会在图表上绘制自己的文本。便签在被悬停、选中或编辑之前只绘制其图钉，与参考平台的便签一致，除非设置了 `tool_options.projection_annotation.always_show_text`（仅在设置时序列化）；该族的 `reveals_on_focus` 钩子会指明这样的绘图，因此当它获得或失去悬停或选择时，帧构建会重建保留的绘图层，而其他所有悬停与选择变化仍只会重新组装保留的几何。剔除边距把空文本计为一行，即其编辑器保留的插入符行。
<!-- B8: projection_annotations — end -->

### 形态、艾略特波浪与周期

<!-- B8: patterns_elliott_cycles — begin -->
“形态、艾略特波浪与周期”族（`kinds/patterns_elliott_cycles.rs`，wire id 160..=191）提供 `xabcd_pattern`、`cypher_pattern`、`abcd_pattern`、`head_and_shoulders`、`triangle_pattern`、`three_drives_pattern`、`elliott_impulse_wave`、`elliott_correction_wave`、`elliott_triangle_wave`、`elliott_double_combo`、`elliott_triple_combo`、`cyclic_lines`、`time_cycles` 和 `sine_line`，它们都是 `ClickAnchors` 工具，每个锚点对应一个手柄。形态是穿过各锚点的之字形折线，带框的点标签放置在高点上方和低点下方；XABCD、cypher、ABCD 和 three drives 还会添加虚线连接线，并标注其各段的惯用价格比率（`tool_options.pattern.show_ratios`），每条连接线都绘制在所有标签之下，剔除边距则按该绘图实际的点、比率和波浪标签来度量；XABCD 与 cypher 为各自的两个三角形着色，头肩形态在外侧两腿之间绘制颈线，并相对于颈线为双肩与头部着色，三角形态在顶点位于前方一个形态宽度以内时，将其 A–C 与 B–D 边延伸至顶点（其规格把逻辑边界按该宽度外扩，且绝不按价格剔除）。

艾略特波浪按 `tool_options.pattern.degree` 的记法标注每一浪；带圆圈的级别把圆圈绘制为描边几何，而不是带圈字形，因此没有任何执行器依赖字体覆盖。周期仅在可见窗格范围内解析重复：周期线自较早的锚点向右，时间周期弧与正弦波向两个方向；间距小于 3 CSS px 的重复会合并为定义周期，弧与波各自保持在 16,384 点的网格化预算之内。填充仅在绘图被选中时才是主体目标（矩形的约定）。每个部件都能由任意锚点前缀解析，因此该族设置共享的 `partial_preview`，并从第二个锚点起预览多锚点放置。该族还新增了通用的 `aeris_charts_render::shape::line_intersection`。
<!-- B8: patterns_elliott_cycles — end -->

### 形状

<!-- B8: shapes — begin -->
“形状”族（`kinds/shapes.rs`，wire id 192..=223）提供 `rotated_rectangle`、`ellipse`、`circle`、`triangle`、`arc`、`curve`、`double_curve`、`polyline` 和 `highlighter`。每个形状都从其锚点以调用方 px 解析，因此圆始终是圆形，旋转矩形在任何缩放和位图比例下都保持直角。旋转矩形的前两个锚点是其两条短边的中点，第三个锚点位于一条长边上；椭圆内接于其两个角点，并使用矩形的八个边界手柄进行编辑；圆由其圆心和一个圆周点确定；圆弧从第一个锚点到第二个锚点，并经过第三个锚点（共线时为它们的弦）；曲线和双曲线经过每个锚点（二次曲线的第三个点位于 t = 1/2，三次曲线的第三和第四个点位于 1/3 与 2/3），`extend_left`/`extend_right` 将它们的端点切线延伸到窗格边缘。闭合轮廓从边的中点起笔，使其平头端相接时共线；开放描边（圆弧、曲线、开放折线）通过共享的 `capped_polyline` 携带端帽，端帽沿精确的端点切线方向。

填充使用 `fill_color`，或使用 20% 的描边颜色（矩形的淡色底），仅在绘图被选中时才是主体目标，并且绝不自身重叠：凸区域通过 `fill_convex`，凹区域或自相交区域（闭合折线、三次曲线的弦区域）通过基于 `shape::nonzero_ribbon` 的共享 `fill_polygon`。该网格化是有界工作，没有粗化回退（与下文荧光笔的 tube 不同）：顶点数超过 `shape::MAX_FILL_VERTICES`（2,048）、边的真交叉超过 4,096 次，或输出梯级超过 8,192 个时，都不会产生填充部件，因此该绘图只绘制其轮廓，也没有内部主体目标。命中测试会重建相同的部件，因此绘制与命中一致，且每个后端执行的是同一帧。不会对此做任何上报：绘图没有诊断通道，且 `closed` 可以在创建后切换，因此添加时的校验无法覆盖顶点上限。对于没有交叉的多边形，顶点上限同时也是成对边扫描的唯一限制，且这三个值都是安全限制，而非调优得出的值。该判定针对的是未裁剪的整个多边形，因此一个 3,000 顶点的多边形即使只有 50 个顶点在屏幕上，也会失去其全部填充；在线性价格比例尺上，平移时判定不会改变，而在对数比例尺上，交叉计数可能随缩放变化。

曲线的弦区域至多展平为 1,024 个点，因此实际上只有闭合折线（其顶点数由宿主控制）才会触及这些上限。解除这些上限意味着像 `shape::tube_ribbon` 那样粗化，或在填充前裁剪到窗格，并伴随一致性风险：应针对已被证实的宿主需求并配合 release 基准测试来做，绝不能仅靠调高常量来实现。曲线和圆通过感知裁剪的辅助函数（`shape::flatten_quadratic`/`flatten_cubic`、`EllipseArc::append_clipped_points`）展平，这些函数仅在曲线可能可见之处细化，且至多花费 1,024 个点，因此放大后宽达数千 px 的圆在屏幕上的误差仍在 0.25 px 以内（完全可见的圆弧采用更廉价的均匀弦）。旋转矩形、圆和圆弧会达到由屏幕推导出的距离，没有任何语义框能界定它们，因此保持完整范围；曲线则按插值过冲来外扩其逻辑跨度。同一个形状框服务于两个钩子：该族自己的 `text_box` 为框布局文本提供形状的框（圆用其自身的框，而不是由圆心到圆周的锚点所定的框），共享的 `paint_bounds` 则使其成为这些完整范围工具精确的屏幕剔除框，因此它们只有在靠近视口或指针时才会进入部件构建和精确命中测试。

荧光笔是一次手绘捕获，以共享的 `Tube` 部件绘制：即与路径的距离在其宽度一半以内的区域，带圆角连接与圆形端帽，由 `shape::tube_ribbon` 根据能够到达窗格的各段构建，在单次遍历中按弦容差简化，勾勒轮廓使非零规则恰好得到并集，并网格化为互不重叠的条带。因此它的 40% 琥珀色在每个执行器上每个像素只混合一次，而不会在 GPU 描边三角形重叠处变暗；当一笔描边超出填充上限时，容差翻倍，仅当最粗的尝试也失败之后才回退为普通描边。其命中测试采用到描边的距离。多锚点形状在除最后一个锚点之外的所有锚点都放置完成之前显示共享的放置引导，之后通过指针预览它们自己的部件。通过共享的 `handles` 与 `drag` 钩子，旋转矩形的手柄是其轴线两端，外加位于其长边中点的派生宽度手柄（第三个锚点没有自己的手柄）：宽度手柄把半宽设为其目标到轴线的距离，拖动轴线端点则会按基线宽度围绕新的轴线重新放置第三个锚点，因此旋转轴线绝不会使其塌缩；宽度点存储在其长边的中点上，不做时间磁吸。折线设置共享的 `close_placement`：放置三个顶点后，点击其第一个顶点即可将其闭合并提交。

<!-- B8: shapes — end -->

## Wire id 与注册表

Wire id 按族预留：core 0..=31、lines 32..=47、channels 48..=63、fibonacci 64..=95、pitchforks_gann 96..=127、projection_annotations 128..=159、patterns_elliott_cycles 160..=191、shapes 192..=223；224..=255 尚未分配。一项测试断言每个规格都位于其所属族的范围内，且具有唯一的 wire id 与名称，并且 serde 名称与目录名称一致。

单一列表的注册表为每个族携带一个 `// B8: <family> — begin/end` 块（在 HTML 中为 `<!-- -->`）。每个族只在其自己的块内编辑，因此并行的族工作可以以累加方式合并：

| 文件 | 块 |
| --- | --- |
| `drawings.rs` | `DrawingKind` 变体 |
| `drawings/tools.rs` | `DRAWING_TOOL_SPECS` 条目；`DrawingKind::spec()` 分支 |
| `drawings/kinds/mod.rs` | `mod` 声明；新增的族专属 `DrawingFamily` 钩子及其在 `DrawingFamily::new` 中的默认值（第二个族也需要的钩子，或基础层为每个族新增的钩子，会移出这些块）；wire 范围测试表 |
| `drawing_contract.rs` | `DrawingKindOptions` 变体；`DrawingToolOptions` 字段、`validate` 检查，以及对所捕获数据的 `template_style` 清除 |
| `lib.rs` | 族选项类型的公共再导出 |
| `packages/charts/src/types.ts` | `drawing_kind` 联合类型；`DRAWING_KIND_TO_U8`；`drawing_kind_options`；族选项类型；`drawing_tool_options` |
| `examples/web_demo/index.html` | 绘图工具栏按钮 |
| [`docs/api/drawings.md`](../../api/drawings.md) | 绘图族目录 |
| [`docs/architecture/engine/drawing-families.md`](drawing-families.md) | 族语义段落（见上文） |

`drawing_perf` 原生示例会在其 `families` 组合中测量每个 wire id 为 32 或更大的工具，并按目录中的锚点数放置每个工具，因此族的工作绝不需要编辑它。

## 添加绘图族

族的实现步骤：

1. 添加 `DrawingKind` 变体（serde 名称即规格名称），并在 `kinds/<family>.rs` 中添加 wire id 位于该族范围内的规格，以及 `pub(crate) static FAMILY`，它由 `DrawingFamily::new(build_parts, kind_options)` 构建，并为其赋值所需的可选钩子。在各自的块中声明该模块，并列出规格与 `spec()` 分支。
2. 只通过 `DrawingParts` 并使用共享辅助函数来解析几何；当纯几何具有通用性时，将其添加到 `aeris_charts_render::shape`。绝不在帧、命中测试器、执行器、WASM 或宿主中发出 `Prim`，也不得在这些位置按种类分支。用 `PartContext::point_px` 映射派生点，并按 `PartContext.scale` 比例确定描边、字形和间隙的尺寸。使用现有的放置类别以及 `Anchors`、`Endpoints`、`RectangleBounds` 或 `Position` 手柄模式。派生（非锚点）手柄及其拖动通过共享的 `handles` 与 `drag` 钩子完成，多次点击闭合则通过 `close_placement` 完成，绝不通过各族自己的拖动或放置代码。将绘图自身的 `text` 以 `DrawingParts::text_label` 绘制，基于 `PartContext::text_lines`，这使其无需任何宿主或 WASM 代码即可就地编辑；这样的族会设置 `owns_text`，使通用文本 pass 不会再次绘制同一份 `text`。`owns_text` 目前仍位于 `kinds/mod.rs` 的 projection_annotations 块中，因此第二个设置它的族须先把该字段及其默认值从这些块移到共享钩子（并把其说明从投影段落移到共享钩子列表）。
3. 将族选项放入该模块中的一个带 serde 默认值的结构体，把它的字段添加到 `DrawingToolOptions`（对列表、字符串或数字，还要在 `validate` 中增加 `&& ...` 检查），再添加一个 `DrawingKindOptions` 变体、一个 `lib.rs` 再导出，以及名为 `tool_options.<block>.<field>` 的 schema 描述符。持久化、剪贴板、同步、模板和 WASM 不需要任何族专属代码，唯一的例外是绘图捕获的数据（而非样式）在具名模板中会由 `template_style` 清除。
4. 在各自的块中添加 TypeScript 联合类型成员、wire id、种类选项与选项类型、演示工具栏按钮，以及 `docs/api/drawings.md` 条目；`impl.ts` 会派生其反向 wire 映射。`packages/charts/api/public-api-v1.json` 是一个生成的哈希，每个并行工作流都会修改它；应在合并之后重新生成它（`bun run update:api`），而不是合并它。
5. 测试：wire 范围表条目；族引擎测试，覆盖默认值、已激活工具的放置、DPR 为 1 和 2 以及位图比例互不相同的小数 DPR 下的帧部件、命中测试（包括在超过 20 个绘图时将索引化结果与暴力遍历对照）、带拉直与磁吸的锚点拖动与主体拖动、键盘手柄数量与微调、跨周期切换的时间标识、schema 与种类选项、原子且可撤销的选项补丁、省略默认值的持久化往返、剪贴板与同步，以及绘图自身的文本恰好只绘制一次；以及一个 Playwright 规格（`drawings-<family>.spec.mjs`），覆盖已激活工具的放置、悬停与命中、选项、持久化、剪贴板与同步、工具栏，以及 WebGPU 与 Canvas2D 的一致性。
