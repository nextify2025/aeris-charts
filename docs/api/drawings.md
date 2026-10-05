# 绘图 API

[文档导航](../README.md) · [架构总览](../Architecture.md) · [API 入口](README.md)

实现归属见[绘图架构](../architecture/engine/drawings.md)；导入、版本及大小限制见[兼容性](compatibility.md)。

- [绘图锚点、磁吸与价格基准](#绘图锚点磁吸与价格基准)
- [绘图族](#绘图族)

## 绘图锚点、磁吸与价格基准

每个绘图锚点都具有时间标识。`drawing.points()` 返回 `{logical, price, time}`，其中 `time` 为 UTC 秒数：小数位置在相邻柱的时间之间插值，超出数据范围的位置则按当前柱间隔外推（未来区域矩形的时间标签显示的就是这个外推出的日期）。`chart.add_drawing()` 和 `drawing.set_points()` 接受 `{logical, price}`、`{time, price}` 或两者同时给出；两者同时存在且不一致时，以 `time` 为准，`points()` 的输出可以精确往返。在图表尚无数据时添加的时间锚点保持待定状态，数据到达后再解析。非时间柱图表不报告 `time`。

数据变化时，锚点跟随合并后的时间轴。新旧数据共有的时间戳保持精确映射，同间隔的保留裁剪或窗口平移则保持其按柱数的外推。前置追加的历史数据会按时间重新放置位于旧数据左侧的锚点，因此比较短的分时历史更早的绘图，在宿主分页载入更多柱的同时仍保持其日期。当数据没有共有的时间戳，或间隔发生变化（1m 到 1h 到 1D、先清空再设置、品种重新加载）时，每个锚点都根据其时间在新坐标轴上解析：10:37 落在从 10:00 的小时柱到下一根柱的 37/60 处，在日线数据上则落在当天的那根柱内。撤销/重做历史以及进行中的创建或拖动状态遵循相同的规则。同步载荷（`drawing_sync_payload`）和剪贴板载荷（`copy_drawings`）携带锚点时间，因此接收端图表会在自己的间隔和历史窗口上解析它们。每一次已提交的绘图变更（API 调用、放置、手绘笔画、移动了内容的指针拖动或键盘编辑、文本编辑）都会推进同步修订，因此已同步的单元格会接受下一个载荷。剪贴板载荷与持久化的绘图文档一样有界（至多 10,000 个绘图、250,000 个锚点和 8 MiB）：超出这些上限时 `copy_drawings` 抛出 `resource_limit`，所列绘图均不存在时抛出 `invalid_data`；`clone_drawing` 可复制图表持有的任意绘图。命名模板（`drawing_template`、`apply_drawing_template`）仅携带样式（外加仓位工具的 `position_account_size` 和 `position_risk_percent`）：绝不携带绘图的名称、分组、修订、可见性、锁定、z 序、周期可见性、价格比例尺或文本，因此应用模板只会重设目标的样式，并保留其标识和自身的文本。

`chart.set_drawing_magnet_mode("off" | "weak" | "strong")` 设置持久的工具栏磁吸（默认 `"off"`，即保留历史行为：仅在按住 Ctrl/Cmd 时启用磁吸）。`"strong"` 始终会把放置或编辑中的锚点吸附到指针下方那根柱上最近的已渲染 OHLC 值；`"weak"` 仅在 12 CSS px 范围内吸附（`DRAWING_WEAK_MAGNET_DISTANCE`）。绘图自身的 `magnet` 选项会为该绘图提升模式。按住 Ctrl/Cmd 会切换实际生效的磁吸（未启用时变为 strong，已启用时变为 off）。触控输入没有修饰键，使用图表的模式。键盘微移绝不吸附。

对于格式错误或超出范围的选项补丁，`chart.add_drawing()` 会抛出 `invalid_options`，而不是丢弃这些选项。在拖动进行中执行撤销/重做，会先取消该拖动。键盘编辑（`Enter`、`Tab`、方向键）会循环切换绘图的可编辑手柄（每个锚点、矩形的八个边界手柄、Long/Short Position 的目标、入场、宽度和止损控件，或绘图族放置在其几何上的手柄，见下文各族的说明），并按微移距离移动当前聚焦的手柄。`drawing_handle_count()` 统计这些手柄的数量。每次微移都会实时应用；`Enter` 会把整个键盘编辑作为一个撤销步骤提交，`Escape` 则把绘图恢复为编辑开始时的样子。未移动任何内容的微移（绘图已锁定、绘图无法沿该轴移动、被窗格边缘钳位）不会改变任何内容，并会据此播报。

绘图自身的文本会在图表的内联编辑器中就地编辑，适用于每一种会绘制文本的绘图：文本工具、趋势线的标签、每种线条、通道、斐波那契、叉形线、形态和形状工具的文本（单行；当标签沿线段排布时，文本沿描边旋转），以及下文列出的“投影与标注”工具的文本框（多行）。档位、点和波浪标签、比率与统计信息均为引擎格式化的文本，仍仅可通过选项设置。有九个工具接受 `text`，但从不在图表上绘制或编辑它：`forecast`、`bars_pattern`、`price_range`、`date_range`、`date_and_price_range`、`projection`、`flag_mark`、`icon` 和 `simple_tag`（其 `text` 即价格坐标轴标签）。双击已选中的绘图，或双击未选中绘图的文本（其第一次点击会选中它），或在图表拥有焦点且绘图已选中时按 Enter 或 F2（在其无障碍绘图目标上按 F2，此时 Enter 仍用于几何编辑），即可打开编辑器；已锁定、已隐藏以及按周期隐藏的绘图不会打开它，文本完全位于其窗格绘图区之外的绘图同样不会（引擎在每个宿主和每条路径上都采用这一规则：双击、Enter、F2、放置以及直接开始编辑）。

引擎决定编辑哪段文本以及它所在的位置，因此没有文本的未选中绘图没有可供双击的标签：请先选中它再双击，或按 Enter 或 F2，或通过其选项添加第一个标签（只有趋势线会在悬停时提示 `+ Add text`）。未选中绘图的文本在悬停时响应文本光标，在点击时响应选择，除非该处有位于更上层的绘图或已选中绘图的锚点手柄。输入时实时重绘，按 Enter 或离开编辑器即提交，按 Escape 则恢复原文本。整次编辑为一个撤销步骤，并仅在提交时一次性反映到 `drawing_sync_payload` 中。文本长度以 `MAX_DRAWING_TEXT_BYTES` 为上限（65,536 字节：选项中更长的 `text` 会被拒绝，且不会应用补丁的其余部分，输入则在上限处停止）；文本工具、趋势线标签以及其余所有沿线标签都保持为单行（换行符会变成一个空格），而各族的文本框可容纳多行（Shift+Enter 添加一行，粘贴时插入纯文本）。编辑器是带标签的文本框，通过无障碍 live region 播报其打开和关闭，并把焦点归还到打开它的位置。

放置文本工具、`anchored_text`、`note`、`callout`、`comment`、`signpost` 或 `simple_annotation` 时会立即打开编辑器，插入符位于默认文本之后；提交或按 Escape 都会保留该绘图，即使文本已被清空（只有文本工具在文本为空时会自行移除）。放置 `price_note`、`price_label` 或箭头标记（它们一开始没有自己的文本）不会打开编辑器。

双击仅在点击能够选中该绘图的位置对已选中的绘图生效：其文本、其主体或其某个手柄。第一次点击落在交易对象或警报控件上的一对点击不作用于任何绘图，在绘图保持选中的状态下于其他位置双击同样不作用于任何绘图。

已激活的绘图工具通过点击放置其点，位置在按键抬起之处：单点工具或 `long_position`/`short_position` 预设点击一次，其他固定点数工具每个点点击一次，`path` 或 `polyline` 则反复点击，直到双击或 Enter。释放之前移动了 5 px 或更多（曼哈顿距离）的按下是一次拖动，而不是点击：它不放置任何东西，也不平移图表，工具保持激活。放置第一个点之后，拖动只移动预览，与悬停相同。这是有意的设计：按下-拖动-释放不会创建这些绘图。`brush` 与 `highlighter` 通过按下拖动绘制，文本工具在按下时放置。

绘图工具激活期间，它拥有订单线与持仓线。在线上、在订单标记的数量与读数区域（否则会拖动 `TP`/`SL` 订单）或在标注徽标上的按下都属于工具：在那里点击会放置工具的锚点，而任何按下都不会移动订单或发出交易意图。指针位于它们上方时，光标是工具的十字光标，线不显示悬停高亮，十字光标保持可见。标记的按钮继续以 pointer 光标工作：关闭单元，以及 `TP` 与 `SL` 按钮（在其上按下拖动仍会创建保护单）。成交箭头与十字光标的警报按钮也是如此。一次性工具在提交后失效，或被 Escape 或 `set_drawing_tool(null)` 取消之后，这些线恢复可拖动。激活、取消以及让工具失效的那次提交，会立即更新静止指针下引擎的 `input_cursor()` 与线的悬停状态。

正在放置的绘图的第一个点落下之后，或在绘制手绘笔画期间，Backspace 与 Delete 属于该绘图。每次按键都移除其最近放置的点：`path` 或 `polyline` 的一个顶点，或固定点数工具的一次点击，例如 `trend_line` 的第一个点或 `xabcd_pattern` 的第三个点。预览继续跟随指针，随后的点击会像被移除的点从未放置过一样完成绘图。没有剩余的点时，这两个键在 Escape 或下一次点击之前不做任何事，因此按住的按键绝不会删除另一个绘图、移除指标或请求宿主移除系列，十字光标也会重新显示以瞄准那次点击。在第一次点击之前，它们照常作用于所选的绘图、指标或系列，工具保持激活。放置期间的撤销（Ctrl/Cmd+Z）保持不变。在绘图自身的无障碍焦点目标上按键仍作用于该绘图，因为聚焦它就是选择了它。

在双击打开编辑器之后，宿主的 `dbl_click` 订阅者仍会运行。因此，把双击绑定到自己设置面板的宿主会同时看到两者：处理程序运行时编辑器已经打开，而在面板控件上调用 `focus()` 会将其关闭（编辑器按原文本提交，不记录撤销步骤，也不产生同步修订），并让焦点停留在该控件上，因此绘图与之前完全一致。编辑器打开期间点击宿主控件会以同样方式将其关闭，并让焦点停留在该控件上；只有 Enter 和 Escape 会把焦点归还到图表内编辑器打开时所在的位置。

绘图的虚线或点线 `style` 在 WebGPU、Canvas2D、GPUI 和原生渲染上绘制出相同的虚线，通用系列的 `line_style` 亦然：引擎会在任何后端绘制之前，把这些描边拆分为虚线段。

### 价格基准（复权）切换

引擎没有复权因子模型；调整后的 OHLC 由宿主计算。绘图通过三次调用跟随基准切换：

1. 以新基准替换系列数据（`series.set_data()`）；指标会重新计算。
2. 调用 `chart.rescale_drawing_prices(segments, basis_label)`。每个分段为 `{from_time?, to_time?, factor}`（UTC 秒数，`[from, to)`，互不重叠，factor 取 1e-6..1e6）。时间落在某分段内的每个锚点价格都会乘以该因子（在 Tick 柱、成交量柱或区间柱图表上，锚点的时间是其所在柱的开盘时间）；Long/Short Position 的各价位使用入场锚点所在的分段，江恩扇形线或固定方格的 `scale_ratio`（每根柱的价格）随其第一个锚点所在的分段缩放。这是数据基准的变更，而非编辑：它同样适用于已锁定的绘图，会以新基准重写撤销/重做历史，并且不记录撤销步骤，因此撤销绝不会恢复旧基准的价格。重新缩放是原子的：分段无效，或因子会使任何价格超出受支持的数值范围，则不会改变任何内容。标签参数会在同一步骤中设置基准。仓位进度会针对新的 K 线重新评估。
3. 保持 `chart.drawing_price_basis()` 同步（`set_drawing_price_basis()` 也可单独设置它）。该标签会被持久化，并随同步和剪贴板载荷携带。在恢复或同步之后，将其与数据基准比较，两者不一致时进行重新缩放。

对于前复权 ↔ 不复权的切换，分段即除权日区间，取每个区间的累计因子（例如 1 拆 2 的拆股之后为 `{to_time: ex_date, factor: 0.5}`）。`chart.set_drawings_points([{drawing, points}])` 会以一个撤销步骤原子地改写多个绘图的锚点，用于宿主计算出的编辑。价格线、警报、标记和交易对象仍归宿主所有；宿主自行改写它们。图表发出的交易意图（来自 Long/Short Position 的括号订单、订单拖动）携带的是显示基准价格，因此在非原始基准下，宿主必须先把它们转换为原始价格，再提交给券商。

## 绘图族

B8 绘图族扩展了 `drawing_kind` 目录。其工具与其他所有绘图使用相同的放置、选择、手柄、拖动、磁吸、键盘编辑、锚点时间标识、历史、持久化、剪贴板、同步和 schema API。部分工具会在其几何上、锚点之外增加手柄（在各族中分别列出）；这些手柄的拖动、磁吸和键盘微移与锚点手柄相同，每次拖动以及每个键盘编辑会话都是一个撤销步骤，其中包括它所编辑的任何选项。各族专属的选项按族各占 `options.tool_options` 下的一个块；补丁会对其进行深度合并（缺失的键保留其值，`null` 会重置一个块，无效的块会以 `invalid_options` 拒绝整个补丁）。schema 描述符使用 `tool_options.line.stats_position` 这样的点分路径命名这些选项，`drawing_kind_options()` 返回解析后的块。种类默认值（例如射线的 `extend_right`）即 schema 默认值，不会写入持久化。

<!-- B8: lines — begin -->
### 线条

- `ray`、`extended_line`、`info_line`、`trend_angle` 和 `arrow_line` 放置两个锚点。在这些工具上，`extend_left` 向第一个锚点之外延伸，`extend_right` 向第二个锚点之外延伸，均沿直线自身方向延伸至窗格边缘；射线默认为 `extend_right`，延长线默认为两者都启用。端帽（`stroke_start`、`stroke_end`）仅绘制在未延伸的端点上；箭头线将 `stroke_end` 默认为 `"arrow"`。`text` 标签像趋势线的标签一样沿线段排布，并以同样方式就地编辑；只有趋势线会在悬停时提示 `+ Add text`。
- 可见的 `labels` 渲染为一个统计框：第一行为价格、价格变化、百分比变化和 tick 数；下一行为柱数、时间范围和持续时间；最后一行为屏幕角度和 CSS px 距离。数值使用绘图比例尺的价格格式化器和锚点的时间标识。`info_line` 默认启用价格变化、百分比变化、柱数、持续时间和角度。`tool_options.line.stats_position`（`"start"`、`"middle"`、`"end"`；默认 `"end"`）把该框放置在第一个锚点之外、中点下方或第二个锚点之外。该框是选择和拖动的主体目标。`volume_in_range` 不渲染任何内容，因为绘图不携带成交量来源。
- `trend_angle` 会增加一条指向第二个锚点的水平虚线参考线、连接到线段的圆弧，以及以度为单位的屏幕角度（上升为正，-90 至 90）。
- `cross_line` 放置一个锚点，并绘制穿过该锚点的全幅水平线和垂直线，水平线的价格标签显示在坐标轴上。其主体可沿两个坐标轴拖动。
- `horizontal_segment` 使两个锚点保持在同一价格上，`vertical_ray` 和 `vertical_segment` 则使两个锚点保持在同一根柱上。放置、拖动或提供某个锚点时，会把共享坐标移动到另一个锚点上，该坐标取自最后放置或拖动的那个锚点，因此提供或导入的不一致锚点对会以同样方式被修复。垂直射线默认为 `extend_right`，使其从第一个锚点穿过第二个锚点，延伸到第二个锚点一侧的窗格边缘。
- `price_line` 放置一个锚点，并绘制一条从该锚点到窗格右边缘的清晰线条，锚点价格印在线条起点上方，并标注在价格坐标轴上（KLineChart 的价格线）。其主体即射线。它自身的 `text` 是通用线条标签，放置方式与水平射线的相同，不会取代价格。
- `drawing_kind_options()` 对每个线条工具返回 `{ kind: "line", stats_position }`。
<!-- B8: lines — end -->
<!-- B8: channels — begin -->
### 通道

- `parallel_channel`、`flat_top_bottom` 和 `disjoint_channel` 放置三个锚点。前两个锚点定义基线。第二条线跨越相同的柱，并位于经过第三个锚点的那条线上，无论该锚点位于哪根柱上：平行通道为基线在屏幕上垂直移动后的线（在每种比例尺模式下都平行），平顶/平底通道为位于第三个锚点价格处的水平线，不相交通道为基线斜率取镜像后的线。`extend_left` 和 `extend_right` 分别把两条线及填充延伸到第一个和第二个锚点之外的窗格边缘。放置时，第一次点击后预览基线，第二次点击后预览整个通道。
- `price_channel` 是 KLineChart 的价格通道：经过前两个锚点的基线为中心线，第二条线平行于它并经过第三个锚点，第三条线则在基线另一侧与第二条线镜像对称。它默认启用 `extend_left` 和 `extend_right` 且无填充（`fill_enabled: true` 会为整个带状区域着色），并且没有中间线。
- 线条之间的填充默认开启（`fill_enabled`）；`fill_color` 默认为描边颜色，alpha 为 20%。线条相交处（平顶/平底、不相交通道），填充在交点处汇合。线条是主体目标；填充仅在绘图被选中时才是可拖动表面，与矩形相同。`text` 标签像趋势线的标签一样沿基线排布。通道线忽略 `stroke_start` 和 `stroke_end`。
- `tool_options.channel.middle_line` 在两条线正中间绘制一条 1 px 的虚线（平行通道默认开启，其他通道默认关闭），颜色为 `middle_color`（`""` 表示跟随 `color`）。
- `regression_trend` 放置两个用于选定柱范围的锚点（位置取整，含两端）；其主体和手柄仅沿时间方向移动。引擎对这些柱上的源系列拟合一条最小二乘直线——源系列是添加到该绘图所在窗格和价格比例尺上、仍然有效的第一个普通系列（指标输出和自定义系列绝不符合条件，足迹图系列和 feature 系列则通过其 OHLC 投影符合条件；重新排序或隐藏系列不会改变源，移除并重新添加其他系列也不会）——并以虚线绘制该直线（`middle_line`、`middle_color`），在其上下分别绘制与之相距 `upper_deviation`（默认 2）和 `lower_deviation`（默认 -2）个残差标准差的线（样本标准差，`n − 1`），二者分别由 `use_upper_deviation` 和 `use_lower_deviation` 开关控制，两线之间的区域被填充；除非 `show_pearsons` 为 false，否则在起点下方显示 Pearson's R（柱位置与数值之间的带符号相关系数，四位小数）。`source` 选择柱的取值（`indicator_input_source`，默认 `"close"`）。这些线跟随源的流式更新；替换最新一根柱或追加柱，其开销只与发生变化的行相关，而与锚定的范围无关。在 as-of（`time_alignment: "as_of"`）源上，拟合对范围内源自身的每根柱只读取一次，而不是重复的坐标轴点。没有源柱的范围会绘制锚点之间的虚线段。锚点的价格会被存储，但不会影响这些线的形状，`text` 标签位于锚点的框内。默认宽度为 1；其他通道默认为 2。
- 手柄位于已绘制的线条上，按锚点顺序每个锚点一个（`drawing_handle_count` 为 3 或 2）：基线的两个端点，对应第三个锚点的第二条线的中点（拖动或微移它会移动第二条线），以及回归线的两个端点，二者跟随拟合结果（放置过程中同样如此）。按住 Shift 拖动基线端点会像趋势线那样把基线拉直。
- 来自 `drawing_template()` 的模板在应用时会替换绘图的 `tool_options.channel`，因此模板中保持默认值的选项也会被重置。
- `drawing_kind_options()` 对三种通过点击放置的通道返回 `{ kind: "channel", middle_line, middle_color }`，并返回带有所有解析后回归选项的 `{ kind: "regression_trend", ... }`。`tool_options.channel` 仅存储已设置的字段。
<!-- B8: channels — end -->
<!-- B8: fibonacci — begin -->
### 斐波那契

| 工具 | 锚点 | 几何 |
| --- | --- | --- |
| `fib_retracement` | 2 | 锚点价格之间的水平档位：0 档在第二个锚点上，1 档在第一个锚点上，其外为延伸档位。档位横跨锚点的时间。 |
| `trend_based_fib_extension` | 3 | 第一段走势的幅度从第三个锚点起投影（0 档在第三个锚点上，1 档距其一整段走势幅度），宽度为第一段的宽度，自第三个锚点起算。 |
| `fib_channel` | 3 | 与第一段平行的线；1 档经过第三个锚点。 |
| `fib_time_zone` | 2 | 全高线条，位于距第一个锚点为锚点时间间距 0、1、2、3、5、8、13、21、34、55、89 倍的位置。 |
| `trend_based_fib_time` | 3 | 全高线条，位于距第三个锚点为第一段持续时间的各比率倍数处。 |
| `fib_speed_resistance_fan` | 2 | 从第一个锚点出发的射线：按每个价格比率穿过第二个锚点的时间，按每个时间比率穿过其价格，延伸至窗格边缘，另加锚点框内的比率网格。 |
| `fib_speed_resistance_arcs` | 2 | 以第一个锚点为中心的圆弧，半径为比率 × 锚点间的屏幕距离，位于第二个锚点一侧（可选完整圆）。 |
| `fib_circles` | 2 | 以锚点中点为圆心的圆；1 档经过两个锚点。 |
| `fib_spiral` | 2 | 以第一个锚点为中心、经过第二个锚点的黄金螺旋线，每四分之一圈按 φ 倍增长，在屏幕上呈顺时针。 |
| `fib_wedge` | 3 | 以第一个锚点为中心、位于指向第二个和第三个锚点的两条边之间的比率圆弧；1 档位于第二个锚点的距离处。 |

- 档位列表是通用的 `levels` 选项：`value`、`color`、`visible`、`style`（`"solid"`、`"dotted"`、`"dashed"`）、`fill_between`、可选的 `fill_color` 以及 `label_visible`，至多 64 个档位。回撤、延伸和通道默认使用 TradingView 可见的回撤档位 0、0.236、0.382、0.5、0.618、0.786、1、1.618、2.618、3.618、4.236 及其调色板（0 和 1 为灰色，0.236 为红色，0.382 为橙色，0.5 为绿色，0.618 为青绿色，0.786 为青色，1.618 为蓝色，2.618 为红色，3.618 为紫色，4.236 为粉色）。其他工具使用惯例表：基于趋势的时间为 0、0.382、0.5、0.618、1、1.382、1.618、2、2.382、2.618、3；扇形线为 0、0.25、0.382、0.5、0.618、0.75、1；圆弧和圆为 0.236 至 4.236；楔形为 0.236 至 1。取自回撤表的值保留其颜色；其他值按列表顺序取用调色板。螺旋线没有档位。
- 可见档位按值排序。`fill_enabled` 是背景开关（除时间区和螺旋线外默认开启）；当上方档位的 `fill_between` 开启时，两个相邻档位之间的带取上方档位的 `fill_color`，或其颜色的 20% 不透明度。带仅在绘图处于选中状态时才能选中并拖动该绘图；档位线、趋势线和标签则始终可以。
- 绘图自身的 `color`、`width` 和 `style`（默认 `#787b86`、1 px、虚线）用于穿过锚点的趋势线、扇形线的网格以及楔形的边（实线）；螺旋线也用它们绘制。档位线使用各档位自己的颜色和样式，宽度取绘图的宽度。
- `extend_left` 和 `extend_right` 把回撤、延伸和通道的档位延伸到窗格的左右边缘。
- 档位标签显示值，对于回撤和延伸还显示经绘图比例尺格式化器格式化后的价格，例如 `0.618 (102.53)`，颜色与该档位相同。
- `tool_options.fibonacci`（`fibonacci_tool_options`）：`reverse`（交换 0 档和 1 档所在的两端；时间区向后投影；螺旋线逆时针旋转）、`show_levels`、`show_prices`、`levels_as_percent`（`61.8%`）、`log_scale`（价格档位在对数空间中插值）、`trend_line`、`grid`（扇形线）、`full_circles`（圆弧）、`label_h_align` 和 `label_v_align`（价格档位默认为 `"left"`/`"middle"`：位于左端之外、在线上居中；时间档位默认为 `"right"`/`"bottom"`）。每个工具的 schema 只列出它读取的字段及其解析后的默认值。`drawing_kind_options()` 返回 `{ kind: "fibonacci", ... }`，其中每个字段均已解析。
- 剔除遵循已绘制的档位：当锚点被滚动到视口之外时，位于锚点之外的价格档位和时间档位仍使绘图保持可见且可命中。扇形线、圆弧、圆、螺旋线和楔形取决于其锚点之间的屏幕距离，因此仅按窗格进行剔除。
<!-- B8: fibonacci — end -->
<!-- B8: pitchforks_gann — begin -->
### 叉形线与江恩

- `andrews_pitchfork`、`schiff_pitchfork`、`modified_schiff_pitchfork` 与 `inside_pitchfork` 放置三个锚点：枢轴点，然后是叉柄的两端。中线从枢轴点出发（Schiff：位于枢轴点的时间，价格取前两个锚点价格的中间；modified Schiff 与 inside：位于前两个锚点的中点）。它穿过叉柄的中点；对 inside 叉形线则穿过第三个锚点，其叉齿穿过第二个锚点及其关于第三个锚点的反射点。点击之间，一条引导线连接已放置的锚点与指针。除每个锚点各有一个手柄外，叉形线与叉形扇在第二与第三个锚点之间的中点还有第四个手柄，它同时移动这两个锚点（`drawing_handle_count` 4）。
- 叉形线的 `levels` 是以半叉柄宽度为单位的中线偏移：层级 `v` 是中线每一侧的一条叉齿，层级 1 穿过叉柄的两端。默认值：0.25、0.382、0.5、0.618、0.75、1、1.5、1.75 与 2，其中 0.5 与 1 可见；`fill_between` 区域按层级颜色的 20% 填充（`fill_enabled`）；层级标签关闭；中线 `color` 为 `#f23645`；宽度 1。未延伸的线越过叉柄延伸一个中线长度；`extend_left` 与 `extend_right` 使每条线延伸到窗格边缘。枢轴偏移变体在前两个锚点之间增加一条虚线引导线。与矩形内部一样，本族中每个工具的区域填充（叉形线与扇形区域、江恩框区域、方图弧线）仅在绘图被选中时才是拖动目标。
- `pitchfan` 放置三个锚点，绘制相同的层级，表现为从第一个锚点出发、穿过另外两个锚点之间叉柄上各层级点的射线。
- `gann_box` 放置两个角点。其 `levels` 是水平价格层级，`tool_options.gann.time_levels` 是垂直时间层级，二者均表示为相对于从第一个角点起算的框的分数（默认 0、0.25、0.382、0.5、0.618、0.75 与 1，四条边上均填充并标注）。`show_angles` 添加从枢轴角点出发的 `angles` 扇形。它有八个边界手柄，Shift 使其在屏幕上成为正方形。
- `gann_square` 放置两个角点，并绘制 `levels` 网格（默认为每边的五等分）、`angles` 扇形（1×8 到 8×1，1×1 位于对角线上）、围绕枢轴角点的四分之一 `arcs` 弧线（边长的五等分，已填充），以及一个显示价格范围、柱数与每柱价格的框。与江恩框一样，它有八个边界手柄，Shift 使其在屏幕上成为正方形。`gann_square_fixed` 放置一个锚点：方形宽 `size_bars`、价格方向高 `size_bars × scale_ratio`，没有比例时则在屏幕上为正方形。它的第二个手柄，即远端角点，用于调整其大小：角点所在的柱以整柱数设置 `size_bars`（至少为 1），将其拖到锚点下方会设置 `reverse`，有 `scale_ratio` 时角点的价格设置该比例（Shift 保持比例不变）；没有比例时，方形在屏幕上保持正方形，并跟随角点到锚点的较大距离。用键盘微调该角点时，边沿箭头所指方向至少移动一整柱（没有比例时，按箭头所在的轴确定大小），因此无论一根柱有多宽，反复按键都会持续调整大小。
- `gann_fan` 放置两个锚点。其 `levels` 是 1×1 斜率的倍数（默认 1/8、1/4、1/3、1/2、1、2、3、4 与 8，标注为 `8x1` 到 `1x8`，区域已填充）。1×1 线穿过第二个锚点，或在设置 `scale_ratio` 时每柱上升相应数量的价格单位。线默认为射线（`extend_right`）；未延伸时，它们止于锚点所围的框。Shift 将第二个锚点校正为 45°。
- `tool_options.gann`（缺省字段保持其默认值）：

  | 字段 | 工具 | 默认值 |
  | --- | --- | --- |
  | `time_levels` | 江恩框 | 如上，0 … 1 |
  | `angles` | 江恩框（配合 `show_angles`）、江恩方图 | 1/8 … 8，正值 |
  | `arcs` | 江恩方图 | 0.2、0.4、0.6、0.8、1，正值 |
  | `reverse` | 江恩框、江恩方图 | `false`；`true` 从第二个锚点的价格起量（江恩框也从它起计时间），并使固定方形向下增长 |
  | `show_angles` | 江恩框 | `false` |
  | `show_stats` | 江恩方图 | `true` |
  | `scale_ratio` | 江恩扇形、固定方形 | `null`：1×1 的每柱价格，正值 |
  | `size_bars` | 固定方形 | 20，范围 1 到 100000 |

  每个工具的 schema 仅列出它使用的字段。`drawing_kind_options()` 为叉形线与叉形扇返回 `{ kind: "pitchfork", levels }`，为江恩工具返回 `{ kind: "gann", levels, time_levels, angles, arcs, reverse, show_angles, show_stats, scale_ratio, size_bars }`。
<!-- B8: pitchforks_gann — end -->
<!-- B8: projection_annotations — begin -->
### 投影与标注

默认值遵循专业平台的常规外观：绘图的 `color`（除非另有说明，即规范主色）绘制标记、引线与框背景；框内文字为 `text_color`，或与框形成黑/白对比；`box_border_color` 为标注框描边；文字使用图表字号，除非设置了 `text_size`。每个工具各自拥有其 `text`（它不遵循其他工具的 3×3 框标签），并将任何可见的 `labels` 渲染为引擎格式化的统计信息。其中八个工具不绘制文字：其 `text` 会被接受并保留，但从不显示，也不能就地编辑。

- `forecast`（源点、目标点）：一条线段，端帽取自 `stroke_start`/`stroke_end`，在远侧有一个源点圆点和一个源价格框，另有一个目标框，显示变化量与百分比、目标时间和结果。结果取自该绘图的源系列（与回归趋势的规则相同：添加到其窗格与价格比例尺上的第一个有效的普通系列），并跟随其流式更新：当源柱之后的某根柱不晚于目标柱到达目标价格（上升目标取最高价，下降目标取最低价）时，结果为 `Success`（上涨色框）——此处的柱指 as-of 源自身的柱，包括在两个坐标轴点之间折叠的柱；重复源柱的点不是后续柱；当目标柱之后已存在已成交的柱而仍未满足上述条件时，结果为 `Failure`（下跌色框）（位于最新的、可能仍在形成中的柱上的目标保持待定，空白数据行（例如未来的交易时段槽位）不是柱）；待定期间没有结果（使用绘图颜色）。
- `bars_pattern`（两个锚点）：用已激活的工具放置时（该工具会就地预览副本），或由不带 `bars` 的 `add_drawing` 创建时，它会把其锚点柱索引之间的柱（至多 128 根；更长的范围聚合为 128 个 OHLC 桶）复制到 `tool_options.projection_annotation.bars`，并把其锚点固定在副本的框上——第一根被复制的柱位于副本的最高值，最后一根位于最低值——使幽灵图形恰好从其源的位置开始。幽灵图形始终填满其锚点之间的框：移动它会移动副本，锚点则在时间上拉伸它，并按副本完整范围的比例缩放它（价格基准重新缩放会精确地缩放它）。`bars_mode` 为 `"hl_bars"`（默认）、`"oc_bars"`、`"line_open"`、`"line_high"`、`"line_low"` 或 `"line_close"`；`mirrored` 在时间上反转副本，`flipped` 使其在框内上下翻转。粘贴、同步与持久化都会携带被复制的柱；命名模板只保留样式，因此应用模板绝不会替换形态的副本，已激活的工具始终复制其自身的范围。对于 as-of 源，它复制该源自身的柱，每根一次。在任何数据存在之前创建的形态没有副本，绘制一个虚线框。
- `price_range`、`date_range`、`date_and_price_range`（两个锚点）：锚点之间的填充（`fill_enabled` 默认开启；`fill_color`，或绘图颜色的 20%），被测量坐标轴的边缘线，穿过中部、指向第二个锚点的带箭头测量线（`stroke_end` 默认为 `"arrow"`），以及位于被测量一端之外的统计框（日期范围则在其下方）。默认 `labels`：价格变化、百分比变化与 tick 数；柱数与持续时间；或全部五项。锚点吸附到整柱与价格 tick（拖动、键盘微调或移动主体时同样如此）；tick 按品种 tick 或价格带阶梯计数，回退到比例尺的 `min_move`。`"date_price_range"`，即早期构建的写法，会被读作 `"date_and_price_range"`，且从不写出。Shift 点击的快速测量会绘制一个临时的日期与价格范围。
- `projection`（顶点、半径点、价格点）：以顶点为中心的圆形扇区，从穿过半径点的射线到穿过价格点的射线（取较短的转向），已填充（`fill_enabled` 默认开启）并描边。可见的 `labels` 度量从顶点到价格点。放置时，第一次点击将顶点显示为一个手柄，并带一条指向指针的临时线；第二次点击后，扇区通过指针预览，第三次点击提交它。
- `anchored_text`（一个锚点）：固定在窗格位置上的文字。其锚点是窗格分数——`logical` 为 x / 窗格宽度，`price` 为从窗格顶部起的 y / 窗格高度——因此在图表滚动、缩放、重新缩放或切换周期时它保持不动。其锚点不带 `time`（`time` 输入被忽略，仅含时间的锚点被拒绝），分数被钳制到 `0..=1`（在 `add_drawing`、`set_points`、粘贴、同步与拖动中，因此它始终可触及），而含有 `0..=1` 之外分数的 `import_state` 文档则为 `invalid_data`，并且磁吸、粘贴偏移与成组移动均不适用。默认文字为 `"Text"`，在锚点处按左/上对齐；`box_color`/`box_border_color` 如同文字工具一样为其加框。
- `note`（一个锚点）：一枚图钉，其尖端即锚点，文字（默认 `"Note"`）显示在钉头旁的框中。该框在备注被悬停、被选中或正在编辑时显示；`tool_options.projection_annotation.always_show_text`（默认 `false`）使其保持可见。
- `price_note`（两个锚点）：一条从定价点引向第二个锚点处的框的引线，框内显示其价格（以及任何文字）。
- `callout`（尖端、框）：一个文本框（默认 `"Callout"`），按 `text_h_align`/`text_v_align`（默认居中）放置在第二个锚点上，带有指向第一个锚点的指针。
- `comment` 与 `price_label`（一个锚点）：一个对话气泡，其尾部尖端即锚点；评论显示其文字（默认 `"Comment"`），价格标签显示锚点的价格及任何文字。
- `signpost`（一个锚点）：一根从锚点向上伸出的杆，顶端是文字牌（默认 `"Signpost"`）。
- `flag_mark`（一个锚点）：一面立在锚点上的旗帜。
- `arrow_mark_up`、`arrow_mark_down`、`arrow_mark_left`、`arrow_mark_right`（一个锚点）：一个块状箭头，其尖端即锚点，任何文字位于箭尾之后，使用 `text_color` 或箭头颜色。向上默认为上涨色，向下默认为下跌色。
- `simple_tag`（一个锚点）：KLineChart 的 simple tag：一条在锚点价格处横贯整个窗格的虚线，并在价格轴上加标签。绘图有 `text` 时标签显示该文字，否则显示价格；文字不绘制在图表上，因此没有就地编辑器（通过选项中的 `text` 设置）。
- `simple_annotation`（一个锚点）：KLineChart 的 simple annotation：一根虚线杆从锚点升起至一个小头部，`text` 位于头部上方的框中（初始为空）。放置它会打开编辑器。
- `icon`（一个锚点）：`tool_options.projection_annotation.icon`——`"star"`（默认）、`"heart"`、`"check"`、`"cross"`、`"circle"`、`"square"`、`"diamond"`、`"triangle_up"` 或 `"triangle_down"`——以锚点为中心，`icon_size` 为宽度（单位 CSS px，8..128，默认 24），使用绘图颜色。
- 各类默认值（填充、箭头、统计、默认文字、箭头颜色）是 schema 默认值，持久化时省略；被清空的默认文字持久化为 `""`。
- `anchored_text`、`note`、`price_note`、`callout`、`comment`、`price_label`、`signpost`、`simple_annotation` 与箭头标记的文字可就地编辑（参见上文的就地文字编辑）；价格备注与价格标签将其价格行保留在文字上方。被清空的框在编辑期间保留一个插入符行。放置 `anchored_text`、`note`、`callout`、`comment`、`signpost` 或 `simple_annotation` 会打开编辑器（在工具带有默认文字时，对默认文字进行编辑）；放置 `price_note`、`price_label` 或箭头标记则不会。
- `drawing_kind_options()` 为本族的每个工具返回 `{ kind: "projection_annotation", bars_mode, mirrored, flipped, pattern_bars, icon, icon_size, always_show_text }`。
<!-- B8: projection_annotations — end -->
<!-- B8: patterns_elliott_cycles — begin -->
### 形态、艾略特波浪与周期

每个工具通过点击放置固定数量的锚点，在放置过程中预览已放置的各段，并为每个锚点提供一个手柄。默认值遵循 TradingView：各工具各有颜色，宽度为 2（循环线为 1），且在 `fill_color` 为空时，区域填充为绘图颜色的 15%。点标签与比例标签使用绘图的字号、字重与斜体（`text_color` 会覆盖其对比色默认值）；它们是主体目标。

| 工具 | 锚点 | 默认颜色 | 绘制内容 |
| --- | --- | --- | --- |
| `xabcd_pattern` | X, A, B, C, D | `#2962FF` | 各段、带阴影的 XAB 与 BCD、比例 AB/XA、BC/AB、CD/BC、AD/XA |
| `cypher_pattern` | X, A, B, C, D | `#2962FF` | 各段、带阴影的 XAB 与 BCD、比例 AB/XA、XC/XA、CD/XC |
| `abcd_pattern` | A, B, C, D | `#089981` | 各段、比例 BC/AB 与 CD/BC |
| `head_and_shoulders` | 基点、左肩、颈点、头部、颈点、右肩、基点 | `#089981` | 各段、位于外侧两段之间的颈线、带阴影的双肩与头部、各部位标签 |
| `triangle_pattern` | A、B、C、D（高点与低点交替） | `#673AB7` | 各段、A–C 与 B–D 两边（当其顶点位于前方且在一个形态宽度之内时延伸至顶点）、带阴影的三角形 |
| `three_drives_pattern` | 起点、驱动 1、回撤、驱动 2、回撤、驱动 3、终点 | `#673AB7` | 各段、标注为 1–3 的驱动段、每段相对于其前一段的比例 |
| `elliott_impulse_wave` | 0, 1, 2, 3, 4, 5 | `#3D85C6` | 各浪标注为 1–5 |
| `elliott_correction_wave` | 0, A, B, C | `#3D85C6` | 各浪标注为 A–C |
| `elliott_triangle_wave` | 0, A, B, C, D, E | `#FF9800` | 各浪标注为 A–E |
| `elliott_double_combo` | 0, W, X, Y | `#6AA84F` | 各浪标注为 W、X、Y |
| `elliott_triple_combo` | 0, W, X, Y, X, Z | `#6AA84F` | 各浪标注为 W、X、Y、X、Z |
| `cyclic_lines` | 周期起点、周期终点 | `#80CCDB` | 虚线连接线，以及从较早的锚点起至右边缘、每隔一个间隔绘制一条的全高垂直线 |
| `time_cycles` | 周期起点（底边）、周期终点（决定拱高） | `#159980` | 宽度与高度同锚点的半椭圆拱，向两侧重复，带阴影 |
| `sine_line` | 一个波峰或波谷、下一个相反的极值 | `#159980` | 穿过两个锚点并贯穿窗格的正弦曲线 |

- 比例为价格比例，以三位小数印在虚线连接线上；`tool_options.pattern.show_ratios: false` 隐藏连接线与比例。
- `tool_options.pattern.degree` 选择艾略特波浪级别：`"supermillennium"`、`"millennium"`、`"submillennium"`、`"grand_supercycle"`、`"supercycle"`、`"cycle"`、`"primary"`、`"intermediate"`（默认）、`"minor"`、`"minute"`、`"minuette"` 或 `"subminuette"`。第 3 浪与 C 浪依次显示为 `{III}`/`{c}`、`[III]`/`[c]`、`<III>`/`<c>`、带圈的 `III`/`c`、`(III)`/`(c)`、`III`/`c`、带圈的 `3`/`C`、`(3)`/`(C)`、`3`/`C`、带圈的 `iii`/`c`、`(iii)`/`(c)` 与 `iii`/`c`。`tool_options.pattern.show_wave: false` 则只保留标签。
- 间隔小于 3 CSS px 的周期重复会折叠为定义周期。
- `fill_enabled` 与 `fill_color` 为 XABCD、cypher、头肩形、三角形形态与时间周期着色；区域填充仅在绘图被选中时才是主体目标。`extend_left`、`extend_right`、`stroke_start` 与 `stroke_end` 不适用于这些工具，而填充不适用于其他工具。`text` 标签相对于锚点所围的框放置。
- `drawing_kind_options()` 为 XABCD、cypher、ABCD 与三驱动返回 `{ kind: "pattern", show_ratios }`，为艾略特工具返回 `{ kind: "elliott_wave", degree, show_wave }`，为头肩形、三角形形态与周期工具返回 `{ kind: "generic" }`。
<!-- B8: patterns_elliott_cycles — end -->
<!-- B8: shapes — begin -->
### 形状

- `rotated_rectangle` 放置三个锚点：两条短边的中点，然后是长边上的一点，其到该轴的距离决定宽度。它在任何缩放下在屏幕上都保持直角。其手柄是两个轴端点，以及位于每条长边中点的宽度手柄（`drawing_handle_count` 4）；第三个锚点没有自己的手柄。宽度手柄将宽度设置为其到轴的距离，拖动轴端点则在屏幕上保持宽度不变，因此旋转矩形绝不会将其压扁。宽度拖动之后，第三个锚点位于其所在长边的中点。
- `ellipse` 放置两个框角点，内接于它们的框；它用矩形的八个手柄编辑，Shift 使框保持正方形（即圆）。`circle` 放置其圆心与圆周上的一点。`triangle` 放置三个顶点。
- `arc` 放置其起点、终点与它经过的一点（共线的点得到直线弦）。`curve` 放置其起点、终点与它在中间经过的点；`double_curve` 放置其起点、终点与它在三分之一和三分之二处经过的点。每个手柄都位于曲线上，`extend_left`/`extend_right` 将曲线端点的切线延续到窗格边缘。放置三锚点或四锚点形状时，到目前为止已点击的锚点与指针以绘图描边的折线显示，直到除最后一个之外的所有锚点都已放置；之后形状本身通过指针预览，直到最后一次点击提交它。
- `polyline` 像 `path` 一样放置顶点：点击添加，双击或 Enter 结束，Backspace 或 Delete 删除最新的顶点，Escape 取消。放置三个顶点后，再次点击第一个顶点会以闭合方式结束折线（指针位于其上方时，预览会合拢）。`tool_options.shape.closed`（默认 `false`）将最后一个顶点与第一个顶点相连，并按非零规则填充所围区域。填充是有界工作：顶点数超过 2,048 的闭合折线，或自交程度严重到其填充超出三角剖分界限的闭合折线，只绘制其轮廓，不填充，也没有内部选择目标（其描边仍可选中它）。这不是错误，所有顶点都会保留，并且在每个后端上完全一致；跟随数千根图表数据柱的区域应属于系列，而不是绘图折线。
- `highlighter` 与 `brush` 一样是自由手绘拖动：一条 20 px 的记号笔笔画，40% 琥珀色，圆形端点。在每个后端上，它在自身重叠处都保持单一不透明度，并忽略 `style`、端帽与填充。
- 描边默认为 2 px。旋转矩形、椭圆、圆、三角形、圆弧与折线将 `fill_enabled` 默认为 `true`，并以 `fill_color` 填充，未设置时使用描边颜色的 20% 不透明度；圆弧填充圆弧与其弦之间的弓形，曲线一旦启用即填充曲线与其弦之间的区域，开放折线从不填充。形状的填充仅在其被选中时才可选中并拖动它，因此未选中形状的内部仍会平移图表。端帽（`stroke_start`、`stroke_end`）适用于开放形状：圆弧、曲线与开放折线。
- 框文字（`text`）相对于形状自身的框对齐，例如圆自身的框，而不是其圆心与圆周锚点所围的框。
- `drawing_kind_options()` 为形状族的每个工具返回 `{ kind: "shape", closed }`；`tool_options.shape.closed` schema 描述符仅为 `polyline` 列出。
<!-- B8: shapes — end -->

### KLineChart overlay 的等价项

从 KLineChart 迁移的宿主可在此找到其每个绘图 overlay。其中七个是独立的工具（wire id 38..=41、52、147 与 148）；其余则是带选项的现有工具，表格说明了具体做法。

| KLineChart overlay | Aeris 工具 |
|---|---|
| `straightLine` | `extended_line` |
| `rayLine` | `ray` |
| `horizontalSegment` | `horizontal_segment` |
| `verticalRayLine` | `vertical_ray` |
| `verticalSegment` | `vertical_segment` |
| `parallelStraightLine` | `parallel_channel`，设置 `extend_left` 与 `extend_right`、`fill_enabled: false` 以及 `tool_options.channel.middle_line: false` |
| `priceChannelLine` | `price_channel` |
| `fibonacciLine` | `fib_retracement`，设置 `extend_left` 与 `extend_right`（层级横跨窗格） |
| `priceLine` | `price_line` |
| `simpleTag` | `simple_tag` |
| `simpleAnnotation` | `simple_annotation` |
