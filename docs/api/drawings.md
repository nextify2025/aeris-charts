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

绘图自身的文本会在图表的内联编辑器中就地编辑，适用于每一种会绘制文本的绘图：文本工具、趋势线的标签、文本标注（`note`、`comment`、`callout`、`price_note`、`anchored_text`）、其余每个目录工具的文本（单行；当标签沿线段排布时，文本沿描边旋转），以及 `simple_annotation` 框（多行）。档位、顶点和波浪标签、比率与统计信息均为引擎格式化的文本，仍仅可通过选项设置。测量工具与 `simple_tag`（其 `text` 即价格坐标轴标签）接受 `text`，但从不在图表上绘制或编辑它。双击已选中的绘图，或双击未选中绘图的文本（其第一次点击会选中它），或在图表拥有焦点且绘图已选中时按 Enter 或 F2（在其无障碍绘图目标上按 F2，此时 Enter 仍用于几何编辑），即可打开编辑器；已锁定、已隐藏以及按周期隐藏的绘图不会打开它，文本完全位于其窗格绘图区之外的绘图同样不会（引擎在每个宿主和每条路径上都采用这一规则：双击、Enter、F2、放置以及直接开始编辑）。

引擎决定编辑哪段文本以及它所在的位置，因此没有文本的未选中绘图没有可供双击的标签：请先选中它再双击，或按 Enter 或 F2，或通过其选项添加第一个标签（只有趋势线会在悬停时提示 `+ Add text`）。未选中绘图的文本在悬停时响应文本光标，在点击时响应选择，除非该处有位于更上层的绘图或已选中绘图的锚点手柄。输入时实时重绘，按 Enter 或离开编辑器即提交，按 Escape 则恢复原文本。整次编辑为一个撤销步骤，并仅在提交时一次性反映到 `drawing_sync_payload` 中。文本长度以 `MAX_DRAWING_TEXT_BYTES` 为上限（65,536 字节：选项中更长的 `text` 会被拒绝，且不会应用补丁的其余部分，输入则在上限处停止）；文本工具、趋势线标签、文本标注以及其余所有沿线标签都保持为单行（换行符会变成一个空格），而 `simple_annotation` 框可容纳多行（Shift+Enter 添加一行，粘贴时插入纯文本）。编辑器是带标签的文本框，通过无障碍 live region 播报其打开和关闭，并把焦点归还到打开它的位置。

放置文本工具、文本标注或 `simple_annotation` 时会立即打开编辑器；对已选中的文本工具或文本标注单击一次，若其文本为空或点击落在其文本上，则会重新打开编辑器。提交或按 Escape 都会保留该绘图，即使文本已被清空；但文本工具和文本标注在文本为空时会自行移除。

双击仅在点击能够选中该绘图的位置对已选中的绘图生效：其文本、其主体或其某个手柄。第一次点击落在交易对象或警报控件上的一对点击不作用于任何绘图，在绘图保持选中的状态下于其他位置双击同样不作用于任何绘图。

已激活的绘图工具通过点击放置其点，位置在按键抬起之处：单点工具或 `long_position`/`short_position` 预设点击一次，其他固定点数工具每个点点击一次，`path` 或 `polyline` 则反复点击，直到双击或 Enter；`polyline` 放置三个或更多顶点后，点击其第一个顶点即闭合并完成（悬停在该顶点上时预览吸附到它上面）。释放之前移动了 5 px 或更多（曼哈顿距离）的按下是一次拖动，而不是点击：它不放置任何东西，也不平移图表，工具保持激活。放置第一个点之后，拖动只移动预览，与悬停相同。这是有意的设计：按下-拖动-释放不会创建这些绘图。`brush` 与 `highlighter` 通过按下拖动绘制，文本工具在按下时放置。

绘图工具激活期间，它拥有订单线与持仓线。在线上、在订单标记的数量与读数区域（否则会拖动 `TP`/`SL` 订单）或在标注徽标上的按下都属于工具：在那里点击会放置工具的锚点，而任何按下都不会移动订单或发出交易意图。指针位于它们上方时，光标是工具的十字光标，线不显示悬停高亮，十字光标保持可见。标记的按钮继续以 pointer 光标工作：关闭单元，以及 `TP` 与 `SL` 按钮（在其上按下拖动仍会创建保护单）。成交箭头与十字光标的警报按钮也是如此。一次性工具在提交后失效，或被 Escape 或 `set_drawing_tool(null)` 取消之后，这些线恢复可拖动。激活、取消以及让工具失效的那次提交，会立即更新静止指针下引擎的 `input_cursor()` 与线的悬停状态。

正在放置的绘图的第一个点落下之后，或在绘制手绘笔画期间，Backspace 与 Delete 属于该绘图。每次按键都移除其最近放置的点：`path` 或 `polyline` 的一个顶点，或固定点数工具的一次点击，例如 `trend_line` 的第一个点或 `pattern_xabcd` 的第三个点。预览继续跟随指针，随后的点击会像被移除的点从未放置过一样完成绘图。没有剩余的点时，这两个键在 Escape 或下一次点击之前不做任何事，因此按住的按键绝不会删除另一个绘图、移除指标或请求宿主移除系列，十字光标也会重新显示以瞄准那次点击。在第一次点击之前，它们照常作用于所选的绘图、指标或系列，工具保持激活。放置期间的撤销（Ctrl/Cmd+Z）保持不变。在绘图自身的无障碍焦点目标上按键仍作用于该绘图，因为聚焦它就是选择了它。

在双击打开编辑器之后，宿主的 `dbl_click` 订阅者仍会运行。因此，把双击绑定到自己设置面板的宿主会同时看到两者：处理程序运行时编辑器已经打开，而在面板控件上调用 `focus()` 会将其关闭（编辑器按原文本提交，不记录撤销步骤，也不产生同步修订），并让焦点停留在该控件上，因此绘图与之前完全一致。编辑器打开期间点击宿主控件会以同样方式将其关闭，并让焦点停留在该控件上；只有 Enter 和 Escape 会把焦点归还到图表内编辑器打开时所在的位置。

绘图的虚线或点线 `style` 在 WebGPU、Canvas2D、GPUI 和原生渲染上绘制出相同的虚线，通用系列的 `line_style` 亦然：引擎会在任何后端绘制之前，把这些描边拆分为虚线段。

### 价格基准（复权）切换

引擎没有复权因子模型；调整后的 OHLC 由宿主计算。绘图通过三次调用跟随基准切换：

1. 以新基准替换系列数据（`series.set_data()`）；指标会重新计算。
2. 调用 `chart.rescale_drawing_prices(segments, basis_label)`。每个分段为 `{from_time?, to_time?, factor}`（UTC 秒数，`[from, to)`，互不重叠，factor 取 1e-6..1e6）。时间落在某分段内的每个锚点价格都会乘以该因子（在 Tick 柱、成交量柱或区间柱图表上，锚点的时间是其所在柱的开盘时间）；Long/Short Position 的各价位使用入场锚点所在的分段；只有锚点价格会被重新缩放。这是数据基准的变更，而非编辑：它同样适用于已锁定的绘图，会以新基准重写撤销/重做历史，并且不记录撤销步骤，因此撤销绝不会恢复旧基准的价格。重新缩放是原子的：分段无效，或因子会使任何价格超出受支持的数值范围，则不会改变任何内容。标签参数会在同一步骤中设置基准。仓位进度会针对新的 K 线重新评估。
3. 保持 `chart.drawing_price_basis()` 同步（`set_drawing_price_basis()` 也可单独设置它）。该标签会被持久化，并随同步和剪贴板载荷携带。在恢复或同步之后，将其与数据基准比较，两者不一致时进行重新缩放。

对于前复权 ↔ 不复权的切换，分段即除权日区间，取每个区间的累计因子（例如 1 拆 2 的拆股之后为 `{to_time: ex_date, factor: 0.5}`）。`chart.set_drawings_points([{drawing, points}])` 会以一个撤销步骤原子地改写多个绘图的锚点，用于宿主计算出的编辑。价格线、警报、标记和交易对象仍归宿主所有；宿主自行改写它们。图表发出的交易意图（来自 Long/Short Position 的括号订单、订单拖动）携带的是显示基准价格，因此在非原始基准下，宿主必须先把它们转换为原始价格，再提交给券商。

## 绘图族

B8 绘图目录以 AerisTerminal 上游的工具外加自有线的七个工具扩展了 `drawing_kind`。每个工具都与其他所有绘图使用相同的放置、选择、手柄、拖动、磁吸、键盘编辑、锚点时间标识、历史、持久化、剪贴板、同步和 schema API，每次拖动以及每个键盘编辑会话都是一个撤销步骤。种类默认值（例如射线的 `extend_right`）即 schema 默认值，不会写入持久化。

**名称。** `drawing_kind` 包含下列规范名称。自有线早期构建的旧拼写（`drawing_kind_alias`）仍保留在该联合类型中，因此不会有导出消失，但它们仅用于输入：`add_drawing`、`set_drawing_tool`、模板、剪贴板与同步载荷以及恢复的文档都接受它们，并通过 `DRAWING_KIND_ALIASES` 将其规范化，而每一项输出（`drawings()`、句柄的 `kind`、导出的文档、载荷）都携带规范名称。`DRAWING_KIND_TO_U8` 只包含规范名称的行（`Record<Exclude<drawing_kind, drawing_kind_alias>, number>`），因此 wire id 总能映射回规范名称；按 wire id 查找之前，请先规范化别名。

| 旧拼写（`drawing_kind_alias`） | 规范种类 |
| --- | --- |
| `date_and_price_range` | `date_price_range` |
| `fib_retracement`、`trend_based_fib_extension`、`fib_channel`、`fib_time_zone`、`trend_based_fib_time` | `fibonacci_retracement`、`fibonacci_extension`、`fibonacci_channel`、`fibonacci_time_zones`、`fibonacci_trend_time` |
| `fib_speed_resistance_fan`、`fib_speed_resistance_arcs`、`fib_circles`、`fib_spiral`、`fib_wedge` | `fibonacci_speed_fan`、`fibonacci_speed_arcs`、`fibonacci_circles`、`fibonacci_spiral`、`fibonacci_wedge` |
| `xabcd_pattern`、`cypher_pattern`、`abcd_pattern`、`head_and_shoulders`、`triangle_pattern`、`three_drives_pattern` | `pattern_xabcd`、`pattern_cypher`、`pattern_abcd`、`pattern_head_shoulders`、`pattern_triangle`、`pattern_three_drives` |
| `elliott_impulse_wave`、`elliott_correction_wave`、`elliott_triangle_wave`、`elliott_double_combo`、`elliott_triple_combo` | `elliott_impulse`、`elliott_correction`、`elliott_triangle`、`elliott_double_combination`、`elliott_triple_combination` |
| `arrow_mark_up`、`arrow_mark_down`、`arrow_mark_left`、`arrow_mark_right` | `arrow_marker_up`、`arrow_marker_down`、`arrow_marker_left`、`arrow_marker_right` |
| `icon` | `icon_stamp`（恢复的文档会把 `icon_name` 设为其内置字形名称，默认为 `"star"`） |
| `flat_top_bottom` | `flat_top_channel`（恢复的文档按第三个锚点的价格选择 `flat_top_channel` 或 `flat_bottom_channel`，水平线穿过基线时选择以该水平线为第二条线的 `disjoint_channel`，其填充在交点处拆成两瓣） |

**Wire id。** id 0 到 84 沿用上游的表（`price_range` 13、`date_range` 14、`date_price_range` 15、`ray` 16 直到 `bars_pattern` 84）；自有线的工具依次占用 240 到 246（`horizontal_segment`、`vertical_ray`、`vertical_segment`、`price_line`、`price_channel`、`simple_tag`、`simple_annotation`）。别名没有 id。id 是 JS/WASM 边界在进程内的细节：文档和载荷携带的是名称。

**选项。** 上游的工具将其选项保留为扁平的绘图选项：列在该工具的 `drawing_property_schema` 中，由每个补丁校验，会被持久化，并由模板、剪贴板与同步载荷携带：

| 选项 | 工具 | 取值与默认值 |
| --- | --- | --- |
| `levels` | 斐波那契、叉形线、叉形扇、江恩框、江恩方图、江恩扇形 | 通用档位列表（`value`、`color`、`visible`、`style`、`fill_between`、`fill_color`、`label_visible`），至多 64 个 |
| `level_reverse` | 档位工具 | `false`；镜像归一化后的档位，把时间区档位移到其起点的另一侧，并对正的江恩扇形比率取倒数 |
| `level_show_prices`、`level_show_values`、`level_show_percents` | 档位工具 | 价格在回撤、延伸与通道上开启；数值在时间区与趋势时间上开启；百分比在除这两者之外的每个档位工具上开启 |
| `level_label_align` | 档位工具 | `"left"`、`"center"`、`"right"`；时间区与趋势时间为 `"left"`，圆弧、圆、螺旋线、楔形、叉形线、叉形扇、江恩框与江恩方图为 `"center"`，其余为 `"right"` |
| `level_log_scale` | 回撤、延伸、通道 | `false`；按几何方式插值正价格 |
| `gann_fans`、`gann_arcs` | `gann_square`、`gann_square_fixed` | 档位列表；扇形线为 1/8、1/4、1/2、1、2、4、8，圆弧为 0.25、0.5、0.75、1 |
| `wave_degree` | 艾略特波浪（按该级别的记法标注，见[形态、艾略特波浪与周期](#形态艾略特波浪与周期)） | `"subminuette"`、`"minuette"`、`"minute"`、`"minor"`（默认）、`"intermediate"`、`"primary"`、`"cycle"`、`"supercycle"`、`"grand_supercycle"`、`"submillennium"`、`"millennium"`、`"supermillennium"` |
| `screen_x`、`screen_y` | `anchored_text` | 窗格分数 0 到 1，默认 0.5 |
| `icon_name`、`icon_size` | `icon_stamp` | 已注册的图像名称（默认为空）；8 到 96 CSS px，默认 24 |
| `bars_pattern_mode`、`bars_pattern_mirror_x`、`bars_pattern_mirror_y` | `bars_pattern` | `"bars"`（默认）、`"oc_bars"`、`"line_open"`、`"line_high"`、`"line_low"`、`"line_close"`（`"hl_bars"` 会被读作 `"bars"`）；镜像为 `false` |
| `regression_source_id`、`regression_deviations` | `regression_trend` | 系列 id 或 `null`（即下文的默认源）；0 到 10，默认 2 |

自有线的工具与测量工具按族各在 `options.tool_options` 下保留一个类型化块（`tool_options.line`、`tool_options.channel`、`tool_options.projection_annotation`）；补丁会对其进行深度合并（缺失的键保留其值，`null` 会重置一个块，无效的块会以 `invalid_options` 拒绝整个补丁），schema 描述符使用 `tool_options.line.stats_position` 这样的点分路径命名这些选项。早期构建的其他块（`tool_options.fibonacci`、`gann`、`pattern`、`shape`，以及回归与柱形态的键）仍会被接受并存储，其中 `tool_options.fibonacci` 由斐波那契工具读取（见[斐波那契](#斐波那契)），`tool_options.pattern` 由形态与艾略特波浪读取（见[形态、艾略特波浪与周期](#形态艾略特波浪与周期)），`tool_options.shape` 由折线读取（见[形状](#形状)）。具有扁平对应项的键，无论选项从何处进入（补丁、模板、粘贴以及恢复的文档），都会按键是否存在迁移到该对应项上，而同一补丁中显式给出的扁平选项优先：斐波那契的 `reverse`、`log_scale`、`show_prices`、`show_levels`、`levels_as_percent` 与 `label_h_align` 分别变为 `level_reverse`、`level_log_scale`、`level_show_prices`、`level_show_values`、`level_show_percents` 与 `level_label_align`（`reverse` 保持其含义：在延伸、通道与时间区上按原样映射，在回撤与速度扇形上则取反，因为早期构建把这两者的 0 档放在第二个锚点上；螺旋线的 `reverse` 会被保留，让没有档位的螺旋线（黄金螺旋）逆时针旋转，从未读取它的工具上的 `reverse` 也会被保留，但不改变任何内容；`label_h_align` 在时间区与趋势时间上互换 `left` 与 `right`，因为早期构建以标签位于线条的哪一侧命名，而 `level_label_align` 以贴靠线条的文字边缘命名（`left` 使标签位于线条右侧））；江恩的 `reverse`、`angles` 与 `arcs` 变为 `level_reverse`、`gann_fans` 与 `gann_arcs`；形态的 `degree` 变为 `wave_degree`；柱形态的 `bars_mode`、`mirrored`、`flipped` 与 `bars` 变为 `bars_pattern_mode`、`bars_pattern_mirror_x`、`bars_pattern_mirror_y` 与快照；图标的 `icon` 与 `icon_size` 变为 `icon_name` 与 `icon_size`（钳制到 96）。回归趋势线的 `tool_options.channel.upper_deviation`、`lower_deviation`、`use_upper_deviation` 与 `use_lower_deviation` 不是别名，而是 `regression_deviations` 的逐侧覆盖：它们被保留并持久化（缺失的一侧跟随 `regression_deviations`），补丁不会因它们改变 `regression_deviations`。没有对应项的键（例如 `tool_options.gann.size_bars`）会被保留并持久化，但在上游工具上不改变任何内容，该工具的 schema 也不列出它们；已在上游工具上重新实现的键例外：上游的六个线条工具读取 `tool_options.line`（见[线条](#线条)），其 schema 列出 `tool_options.line.stats_position`；上游的通道与回归趋势线读取 `tool_options.channel`（见[通道](#通道)），其 schema 列出通道的 `middle_line` 与 `middle_color`，回归另列出 `upper_deviation`、`lower_deviation`（默认 `null`，即跟随 `regression_deviations`）、`use_upper_deviation`、`use_lower_deviation`、`source` 与 `show_pearsons`；斐波那契工具读取 `tool_options.fibonacci`（见[斐波那契](#斐波那契)），其 schema 按工具列出 `trend_line`、`grid`、`full_circles`、`label_v_align` 与螺旋线的 `reverse`；谐波形态读取 `tool_options.pattern.show_ratios`，艾略特波浪读取 `tool_options.pattern.show_wave`，二者默认 `true`，其 schema 分别列出它们；江恩工具读取 `tool_options.gann`（见[叉形线与江恩](#叉形线与江恩)），其 schema 列出江恩框的 `time_levels`、`angles` 与 `show_angles`，两种方图的 `show_stats`，以及江恩扇形与固定方图的 `scale_ratio`；折线读取 `tool_options.shape.closed`（见[形状](#形状)），其 schema 只为折线列出它（默认 `false`）。这些块的默认值即上游的外观（`tool_options.fibonacci.trend_line` 与 `grid`、`tool_options.gann.time_levels` 与 `show_stats` 默认关闭或为空），因此只发送一个键的补丁不会打开其他内容（`tool_options.pattern` 例外：按维护者的决定，谐波比率默认显示，`show_wave` 默认即上游的外观）；`tool_options.projection_annotation` 只写出与默认值不同的字段。早期构建写出的文档恢复时，会把该构建从不写出的默认值显式写入这些块（见[持久化 V1](compatibility.md#持久化-v1)）。

`drawing_kind_options()` 为斐波那契、叉形线、叉形扇、江恩框与江恩扇形工具返回 `{ kind: "levels", levels, reverse, log_scale, show_prices, show_values, show_percents, label_align }`，为两种方图返回 `{ kind: "gann_square", levels, fans, arcs, reverse, show_prices, show_values, show_percents, label_align }`，并返回 `{ kind: "regression_trend", source_id, deviations }`、`{ kind: "elliott", wave_degree }`、`{ kind: "anchored_text", screen_x, screen_y, box_color, box_border_color, box_border_width }`、`{ kind: "icon_stamp", icon_name, icon_size }` 和 `{ kind: "bars_pattern", mirror_x, mirror_y, mode, bar_count }`，为 `note`、`comment`、`callout` 与 `price_note` 返回 `{ kind: "text", ... }`，为自有线的工具与测量工具返回 `{ kind: "line", stats_position }`、`{ kind: "channel", middle_line, middle_color }` 与 `{ kind: "projection_annotation", ... }`，为其余每个工具返回 `{ kind: "generic" }`。

### 线条

- `ray`、`extended_line`、`info_line`、`trend_angle` 和 `arrow_line` 放置两个锚点。射线保持从第一个锚点出发的方向，延长线则向两个方向延伸；引擎把它们投影到窗格边缘。`arrow_line` 将 `stroke_end` 默认为 `"arrow"`。`info_line` 显示其可见的 `labels`（默认为价格变化、百分比变化、柱数和角度），`trend_angle` 显示其角度。标签数值由引擎格式化：`date_time_range` 经柱时间标签打印锚点的时间，`duration` 打印经过的时间（在没有时间的坐标轴上则为柱跨度）。射线始终延伸过第二个锚点，`extend_left` 使它穿过第一个锚点向后延伸到窗格边缘（`extend_right: false` 不会把射线变成线段，延长线的 `extend_*` 也不起作用；需要线段时请用趋势线）。信息线、角度线与箭头线按 `extend_left`/`extend_right` 延伸，竖直放置时延伸到窗格的上下边缘。`text` 标签像趋势线的标签一样沿线段排布，并以同样方式就地编辑；只有趋势线会在悬停时提示 `+ Add text`。
- `cross_line` 放置一个锚点，并绘制穿过该锚点的全幅水平线和垂直线，水平线的价格标签显示在坐标轴上。
- 这六个线条工具（`ray`、`extended_line`、`info_line`、`trend_angle`、`cross_line` 与 `arrow_line`）带有 `tool_options.line` 块时，改用早期构建的外观：可见的 `labels` 绘制为一个由引擎格式化的统计框（价格类、时间类与几何类数值各占一行，同组数值以两个空格分隔，底色为绘图颜色的半透明色，文字为黑色或白色），其位置由 `tool_options.line.stats_position`（`"start"`、`"middle"`、`"end"`；块存在时默认 `"end"`）决定，统计框可悬停与选中，并取代逐项标签；`trend_angle` 增加一条从第一个锚点朝第二个锚点一侧、与线段等长的虚线水平参考线，参考线与线段之间的圆弧，以及圆弧旁折算到 -90° 到 90° 的屏幕角度；`ray`、`extended_line`、`info_line`、`trend_angle` 与 `arrow_line` 只在没有延伸到窗格边缘的端点上绘制 `stroke_start`/`stroke_end` 端帽，箭头下的描边缩回一个描边宽度，端帽本身也可命中。新绘图不带该块，与上游的绘制一致；写入该块（例如 `{ tool_options: { line: {} } }`，或写回 schema 的默认值 `{ stats_position: "end" }`）即开启，`{ tool_options: { line: null } }` 即关闭，`drawing_kind_options()` 对这些工具仍返回 `{ kind: "generic" }`。早期构建写出的文档恢复时带有该块，其信息线保留五项统计（加上持续时间），而新的信息线保留上游的四项。
- `horizontal_segment` 使两个锚点保持在同一价格上，`vertical_ray` 和 `vertical_segment` 则使两个锚点保持在同一根柱上。放置、拖动或提供某个锚点时，会把共享坐标移动到另一个锚点上，该坐标取自最后放置或拖动的那个锚点，因此提供或导入的不一致锚点对会以同样方式被修复。`extend_left` 和 `extend_right` 分别把它们延伸到第一个和第二个锚点之外；垂直射线默认为 `extend_right`，使其从第一个锚点穿过第二个锚点延伸到窗格边缘。可见的 `labels` 渲染为一个统计框（价格、价格变化、百分比变化和 tick 数；柱数、时间范围和持续时间；屏幕角度和 CSS px 距离），其位置由 `tool_options.line.stats_position`（`"start"`、`"middle"`、`"end"`；默认 `"end"`）决定。
- `price_line` 放置一个锚点，并绘制一条从该锚点到窗格右边缘的清晰线条，锚点价格印在线条起点上方，并标注在价格坐标轴上（KLineChart 的价格线）。其主体即射线。它自身的 `text` 是通用线条标签，不会取代价格。

### 通道

- `parallel_channel`、`flat_top_channel` 和 `flat_bottom_channel` 放置三个锚点，`disjoint_channel` 放置四个；引擎为命中测试、填充和描边一次性解析它们的边界，而其锚点仍是可编辑、会被持久化的几何。它们的填充默认开启，`extend_left`/`extend_right` 把两条线及填充延伸到窗格边缘。填充位于两条线按方向配对的端点之间：第二条线反向的不相交通道填满整个四边形，两条线相交时填充在交点处相接的两瓣；选中时，填充按其绘制区域成为拖动面。`tool_options.channel.middle_line`（默认关闭）在两条线之间、线的下方绘制一条 1 px 虚线中线，连接配对端点的中点（两条线的端点位于相同柱上时恰在中间），颜色为 `middle_color`（为空时跟随描边颜色），它随延伸一起延伸，并且是主体命中目标。
- `regression_trend` 放置两个锚点，用于选定一个柱窗口：位置位于两者之间的柱。引擎在该窗口上对其源的有限值（`tool_options.channel.source`：`open`、`high`、`low`、`close`、`hl2`、`hlc3`、`ohlc4` 或 `hlcc4`，默认 `close`）进行拟合，并绘制拟合线，以及与之相距 `regression_deviations` 个总体残差标准差的两侧线与其间的区带。`upper_deviation` 与 `lower_deviation`（带符号，-100 到 100）分别覆盖一侧的偏移，更改 `regression_deviations` 时保留已覆盖的一侧；`use_upper_deviation`/`use_lower_deviation` 为 `false` 时，该侧既没有线也没有区带；两侧位于拟合线同一侧时，区带从拟合线填充到较远的一侧。`middle_line` 把拟合线画成 `middle_color` 的 1 px 虚线，`show_pearsons` 在起点下方绘制皮尔逊 R（保留四位小数，终点在起点左侧时右对齐；不是命中目标），`extend_left`/`extend_right` 把各线与区带延伸到窗格边缘。锚点只选择柱：主体与手柄拖动、磁吸和键盘微调只沿时间移动，选中时手柄位于拟合线在两个锚点柱处的位置，区带是拖动面。有限值少于两个的窗口（未来区域、数据尚未加载、回放时钟早于该窗口）没有拟合，会绘制锚点之间的虚线段，该线段仍可选中。当 `regression_source_id` 所指的系列在该绘图的窗格与价格比例尺上有效时，源即为该系列（在那里无效的 id 会使绘图没有拟合）；否则，源是添加到该绘图的窗格与价格比例尺上、仍然有效的第一个普通系列（指标输出和自定义系列绝不符合条件，足迹图系列和 feature 系列则通过其 OHLC 投影符合条件；重新排序或隐藏系列不会改变源）。拟合只读取回放时钟显示的行，对 as-of（`time_alignment: "as_of"`）源的每根柱只读取一次，并跟随源的流式更新；替换最新一根柱或追加柱，其开销只与发生变化的行相关，而与窗口无关。
- `price_channel` 是 KLineChart 的价格通道：经过前两个锚点的基线为中心线，第二条线平行于它并经过第三个锚点，第三条线则在基线另一侧与第二条线镜像对称。它默认启用 `extend_left` 和 `extend_right` 且无填充（`fill_enabled: true` 会为整个带状区域着色），第三个锚点的手柄位于第二条线的中点，放置时第一次点击后会预览基线。

### 斐波那契

| 工具 | 锚点 | 默认档位 |
| --- | --- | --- |
| `fibonacci_retracement` | 2 | 0、0.236、0.382、0.5、0.618、0.786、1（0 档在第一个锚点上，1 档在第二个锚点上） |
| `fibonacci_extension` | 3 | 0、0.618、1、1.618、2、2.618：第一段走势的幅度从第三个锚点起投影 |
| `fibonacci_channel` | 3 | 同回撤：与第一段平行、朝第三个锚点偏移的线 |
| `fibonacci_time_zones` | 2 | 锚点时间间距的 0、1、2、3、5、8、13、21、34 倍 |
| `fibonacci_trend_time` | 3 | 同时间区，从第三个锚点起投影 |
| `fibonacci_speed_fan` | 2 | 同回撤：从第一个锚点出发的射线 |
| `fibonacci_speed_arcs`、`fibonacci_circles` | 2 | 同回撤：围绕第二个锚点的同心档位 |
| `fibonacci_spiral` | 2 | 同回撤：从第一个锚点出发、有界的对数螺旋圈 |
| `fibonacci_wedge` | 3 | 同回撤：两条侧边射线之间的同心圆弧 |

每个档位控制其可见性、颜色、描边样式、到前一档位的填充及其标签；`fill_enabled` 默认开启。当锚点被滚动到视口之外时，位于锚点之外的价格档位和时间档位仍使绘图保持可见且可命中。档位标签与档位线一样是拖动面（悬停显示移动光标），绘图被选中时，已填充的区带也是拖动面（未选中时在区带上拖动会平移图表）；这对江恩扇形同样成立。`fibonacci_retracement`、`fibonacci_extension` 与 `fibonacci_channel` 的档位按 `extend_left`/`extend_right` 延伸到窗格的左右边缘（通道的档位沿其斜率延伸）。

`tool_options.fibonacci` 中的选项默认都是上游的外观，新绘图不带该块：

| 选项 | 工具 | 效果 |
| --- | --- | --- |
| `trend_line` | 回撤、扩展、时间区、趋势时间、速度弧、圆、螺旋线 | 以绘图自身的描边穿过锚点绘制趋势线（扩展与趋势时间为两段，圆为穿过两个锚点的 1 档直径，螺旋线为 1 CSS px 虚线），可拖动 |
| `grid` | 速度扇形 | 每个可见档位按其比例在锚点框上的一条水平线与一条竖直线（0..1 以外的档位位于框外），可拖动 |
| `full_circles` | 速度弧 | 围绕第二个锚点的完整圆，而不是朝第一个锚点的半圆 |
| `label_v_align` | 回撤、扩展、通道、时间区、趋势时间 | `"top"`（默认，线上方）、`"middle"`（垂直居中于线，放在 `level_label_align` 所指那一端之外，该端延伸到窗格边缘时放在内侧）或 `"bottom"`（线下方）；时间标签位于窗格的顶部、中部或底部 |
| `reverse` | 螺旋线 | 黄金螺旋逆时针旋转 |

块一旦存在（任意键都可以；补丁中的空块 `{}` 会被丢弃），速度弧、圆与楔形的圆环与区带只在窗格可见的部分细分，处处与真实圆相差不超过 0.25 px，虚线跟随圆弧而不随窗格平移；不带该块时保留上游每圈 32 段的折线。`levels` 为空列表的 `fibonacci_spiral` 绘制黄金螺旋：以第一个锚点为中心、经过第二个锚点，每四分之一圈按 φ 增长，默认顺时针，直到离开窗格为止（要让新的螺旋线采用它，宿主在激活工具时传入 `levels: []`）。早期构建写出的文档恢复时带有这些选项的早期默认值：趋势线、扇形网格、价格档位居中于线左侧、时间档位位于窗格底部、精确圆环与黄金螺旋。

### 叉形线与江恩

- `andrews_pitchfork`、`schiff_pitchfork`、`modified_schiff_pitchfork` 与 `inside_pitchfork` 放置三个锚点，并推导出各不相同的中线起点和平行的叉齿；`pitchfan` 以穿过外侧锚点的射线绘制相同的档位。它们的 `levels` 沿第二与第三个锚点之间的叉柄放置叉齿（叉形线默认为 0、0.5 与 1，叉形扇默认为 0、0.25、0.5、0.75 与 1）。
- `gann_box` 放置两个锚点，并以带样式的 `levels`（默认 0 到 1，以八分之一为间隔）解析出价格与时间网格。`gann_square` 在该网格上增加 `gann_fans` 角度射线与 `gann_arcs` 四分之一圆弧；`gann_square_fixed` 由其两个锚点在屏幕上解析出一个正方形框。这三组档位各有独立的 schema、补丁、持久化与档位间填充控制。
- `gann_fan` 放置两个锚点，并从其枢轴向窗格边缘投射九条成比例的角度射线（`levels` 1/8、1/4、1/3、1/2、1、2、3、4、8）。
- 叉形线与叉形扇被选中时，已填充的区带是拖动面（未选中时在区带上拖动会平移图表），江恩框与方图被选中时框外的档位单元格同样如此。叉形线与叉形扇的第四个手柄位于第二与第三个锚点的中点：拖动它（或用键盘微调第四个手柄）同时移动这两个锚点，作为一次撤销。
- 固定方图的第二个手柄位于绘制出的方形对角上，而不是第二个锚点上（锚点可能远在方形之外）。拖动它按整根柱调整边长：没有 `scale_ratio` 时按指针离第一个锚点较大的距离（键盘按移动的轴，每步至少一根柱）确定边长，方形保持屏幕上的正方形，第二个锚点的价格已在新对角之外时保持不变，否则移到对角之外的远处，因此普通的价格轴缩放下边长通常不变（大幅缩小价格轴仍可能让价格边胜出）；有 `scale_ratio` 时对角所在的柱确定边长、对角的价格设定比率（按住 Shift 保持按下时的比率）。撤销与取消会连同锚点一起恢复比率。

`tool_options.gann` 中的选项默认都是上游的外观，新绘图不带该块：

| 选项 | 工具 | 效果 |
| --- | --- | --- |
| `time_levels` | 江恩框 | 框自己的竖直（时间）档位，按框宽的比例自枢轴角起计，在框上方标注；非空时框的 `levels` 只绘制水平线，区带变为重叠的价格区带（横贯框宽）与时间区带（纵贯框高） |
| `angles`、`show_angles` | 江恩框 | `show_angles` 为 `true` 时从枢轴角绘制 `angles`（1×1 的倍数，1×1 即框的对角线），到框边为止，不填充，可拖动 |
| `show_stats` | 江恩方图、固定方图 | 远角旁的统计框：价格范围、柱数与每柱价格（没有宽度时为 0 柱，不显示每柱价格），可拖动 |
| `scale_ratio` | 江恩扇形、固定方图 | 1×1 每根柱的价格（正数，不超过 `MAX_SAFE_VALUE`，即价格基准缩放与对角拖动保持的范围）。扇形的 1×1 以该斜率到达第二个锚点所在的柱（朝第二个锚点价格的方向）；固定方图的对角位于第二个锚点所在的柱、自第一个锚点价格起偏移柱数乘以比率处。`null` 保持锚点斜率与屏幕上的正方形。价格基准缩放（`rescale_drawing_prices`）随锚点一起缩放它 |

早期构建写出的文档恢复时，江恩框带有七个 `time_levels`，两种方图带有 `show_stats: true`；带 `scale_ratio` 的固定方图按比率绘制，与早期构建一致。

### 投影与标注

- `projection`（顶点、目标点）：目标点的时间与价格一同设定时间跨度与投影高度，并以三角形填充。
- `forecast`（入场点、目标点）：目标点处的标签显示以百分比表示的变动，以及状态：当入场柱之后的某根柱（入场柱本身绝不计入）不晚于目标柱到达目标价格（上升目标取最高价，下降目标取最低价）时为 `target reached`；当目标柱之后已存在已成交的柱而仍未满足上述条件时为 `expired`（空白数据行，例如未来的交易时段槽位，不是柱）；否则为 `pending`。在时间坐标轴上，目标柱的时间经柱时间标签打印，位于目标点处标签上方一行。其源即回归趋势的默认源，读取方式也相同（回放时钟、as-of 柱），状态跟随该源的流式更新；状态是派生的，绝不持久化。
- `bars_pattern`（源起点、源终点、目标点）：放置它时，会把前两个锚点之间至多 512 根有限的 OHLC 柱复制到一个快照中，每根柱位于其柱偏移处，因此间隙得以保留。第三个锚点移动这份冻结的副本，而不会再次读取源；移动某个源锚点会在编辑提交时重新捕获副本。`bars_pattern_mode` 绘制柱、开盘价到收盘价的竖线（`"oc_bars"`）或穿过所选价格的线，两个镜像选项会将其翻转。持久化、剪贴板与同步都携带该快照，因此目标图表不需要源数据。
- `price_range`、`date_range` 与 `date_price_range`（两个锚点）：锚点之间的填充（`fill_enabled` 默认开启；`fill_color`，或绘图颜色的 20%），被测量坐标轴的边缘线，穿过中部、指向第二个锚点的带箭头测量线（`stroke_end` 默认为 `"arrow"`），以及位于被测量一端之外的统计框（日期范围则在其下方）。默认 `labels`：价格变化、百分比变化与 tick 数；柱数与持续时间；或全部五项。锚点吸附到整柱与价格 tick（拖动、键盘微移或移动主体时同样如此）；tick 按品种 tick 或价格带阶梯计数，回退到比例尺的 `min_move`。Shift 点击的快速测量会绘制一个临时的日期与价格范围。
- `anchored_text`（一个锚点）：位于固定窗格位置 `screen_x`/`screen_y` 的文字，该位置取自放置时的点击（在添加绘图或设置其锚点时则取自锚点），因此时间与价格比例尺的变化不会移动它；拖动、撤销与坐标补丁会编辑该位置。
- `note` 与 `comment`（一个锚点）、`callout`（尖端、框；一条指向尖端的引线，`stroke_start` 默认为 `"arrow"`）以及 `price_note`（一个定价点，带一条横贯窗格的引导线）是文本标注：它们像文本工具一样在内联编辑器中编辑，并在放置时打开编辑器。`callout` 的尖端和框各有一个手柄；其余工具作为一个主体移动。
- `price_label`（一个锚点）：位于窗格边缘的徽标，显示引擎格式化的价格，除非 `text` 覆盖它。
- `arrow_marker_up`、`arrow_marker_down`、`arrow_marker_left`、`arrow_marker_right` 与 `flag_mark`（一个锚点）以及 `signpost`（两个锚点：底脚与牌面）是带可选杆的填充标记。
- `icon_stamp`（一个锚点）：以 `icon_name` 注册的图像，宽 `icon_size` CSS px。宿主通过 `chart.register_drawing_icon(name, width, height, pixels)` 注册 RGBA8 图像（至多 32 个图像，每个至多 96×96 像素，名称至多 64 字节；其他任何情况都会抛出 `invalid_options`，再次注册同一名称会替换其像素），并通过 `chart.remove_drawing_icon(name)` 移除一个图像；Rust 宿主调用 `ChartEngine::set_drawing_icon(name, width, height, pixels)` 与 `remove_drawing_icon(name)`，二者返回 `bool`。持久化只保留名称，因此恢复之后需要重新注册这些图像。没有已注册图像的名称，若为 `"star"`、`"heart"`、`"check"`、`"cross"`、`"circle"`、`"square"`、`"diamond"`、`"triangle_up"` 或 `"triangle_down"`，则绘制内置的矢量字形，否则绘制一个着色的占位符。
- `simple_tag`（一个锚点）：KLineChart 的简单标签：一条在锚点价格处横贯整个窗格的虚线，并在价格坐标轴上加标签。绘图有 `text` 时标签显示该文字，否则显示价格；文字不绘制在图表上，因此没有内联编辑器。
- `simple_annotation`（一个锚点）：KLineChart 的简单标注：一根虚线杆从锚点升起至一个小头部，`text` 位于头部上方的框中（初始为空，可跨多行，并可就地编辑）。放置它会打开编辑器。

### 形态、艾略特波浪与周期

| 工具 | 锚点（顶点标签；艾略特波浪为默认 `minor` 级别的标注） |
| --- | --- |
| `pattern_xabcd`、`pattern_cypher` | X, A, B, C, D |
| `pattern_abcd` | A, B, C, D |
| `pattern_head_shoulders` | N, LS, N, H, N, RS, N |
| `pattern_triangle` | A, B, C, D, E |
| `pattern_three_drives` | 0, 1, A, 2, B, 3 |
| `elliott_impulse` | 起点（不标注）, 1, 2, 3, 4, 5 |
| `elliott_correction` | 起点（不标注）, A, B, C |
| `elliott_triangle` | 起点（不标注）, A, B, C, D, E |
| `elliott_double_combination` | 起点（不标注）, W, X, Y |
| `elliott_triple_combination` | 起点（不标注）, W, X, Y, X, Z |
| `cyclic_lines`、`time_cycles`、`sine_line` | 2 |

形态与艾略特波浪是有序、可编辑的锚点路径，顶点标签由引擎拥有，位于各顶点上方 8 px，是主体命中目标（悬停显示移动光标，拖动移动绘图）。放置期间，从第二次点击起，预览就是将要提交的绘图本身：已放置的锚点加指针所成的折线、顶点标签、比率与填充。

- 谐波形态（`pattern_xabcd`、`pattern_cypher`、`pattern_abcd`、`pattern_three_drives`）在折线之后绘制 1 px 虚线比率连线，并在每条连线中点绘制一个以绘图颜色为底、保留三位小数的价格比率框（文字为 `text_color`，未设置时为黑色或白色）：XABCD 为 XB 上的 AB/XA、AC 上的 BC/AB、BD 上的 CD/BC 与 XD 上的 AD/XA；cypher 为 XB 上的 AB/XA、AC 上的 XC/XA 与 XD 上的 CD/XC；ABCD 为 AC 上的 BC/AB 与 BD 上的 CD/BC；三驱形态为每一段对前一段的比率，画在跨越这两段的连线上。参考段水平时不绘制该比率。连线与比率框都是主体命中目标，由 `tool_options.pattern.show_ratios`（默认 `true`）控制，schema 列出该键。
- `pattern_xabcd` 与 `pattern_cypher` 在 `fill_enabled` 时为 X-A-B 与 B-C-D 两个三角形着色（`fill_color`，未设置时为绘图颜色、alpha 38，即不透明度约 15%（透明度 85%））；填充仅在绘图被选中时是拖动面。
- `pattern_head_shoulders` 总是绘制其颈线：穿过两个颈部锚点（第三与第五个锚点）的直线，从它与第一段相交处画到它与最后一段相交处（不相交时止于颈部锚点），使用绘图的描边；`fill_enabled` 时为两肩与头部相对于颈线的三个三角形着色。
- `pattern_triangle` 在其前四个锚点上取 A–C 与 B–D 两边。当图形朝右（C、D 位于 A、B 之右）且 `extend_right`，或朝左且 `extend_left` 时，绘制这两边；两边在图形前方一个图形宽度内汇合时画到顶点，否则停在锚点上。`fill_enabled` 为两边之间的区域着色（有顶点时为 A、顶点、B 三角形，否则为凸四边形 A、C、D、B）。新绘图不设置这两个标志，早期构建写出的三角形形态恢复时两者都为 `true`。设置标志时，绘图的时间剔除边界向两侧各扩展一个图形宽度（价格不剔除）。
- 艾略特波浪按 `wave_degree` 的 Frost–Prechter 记法标注各浪，起点（第一个锚点）不标注：`cycle` 及以上用大写罗马数字，`minor` 到 `primary` 用阿拉伯数字，`minute` 及以下用小写罗马数字；字母在阿拉伯数字的三个级别上为大写，其余为小写；每三个级别依次为无括号、括号与圆圈（例如 `minor` 为 `3`，`intermediate` 为 `(3)`，`primary` 为带 1 px 圆圈的 `3`，圆圈下缘位于顶点上方 6 px），`submillennium`、`millennium` 与 `supermillennium` 分别以 `<>`、`[]` 与 `{}` 包围大写罗马数字。圆圈内部也是命中目标。`tool_options.pattern.show_wave: false` 不绘制波浪折线（也不再命中它），只保留标签；schema 列出该键（默认 `true`）。

周期线与时间周期从其两个锚点重复绘制垂直标记（可见的至多 256 个），正弦线把可见窗格采样为至多 512 段。

### 形状

`rotated_rectangle`（一条边与一个深度点）、`ellipse`（两个角点）、`circle`（圆心与圆周点）、`triangle`（三个顶点）、`arc`（起点、经过点、终点）、`curve`（起点、控制点、终点）与 `double_curve`（起点、两个控制点、终点）解析为共享的屏幕几何，用于绘制与命中测试；旋转矩形、椭圆、圆与三角形默认填充。

- 放置与手柄：`ellipse` 与 `rectangle` 一样以其外接框的八个手柄编辑，Shift 在放置与拖动角点时把它拉成圆。`rotated_rectangle` 依次点击一条边的两端与深度；其第三个手柄位于远侧边的中点，近侧边的中点另有一个宽度手柄（`drawing_handle_count` 为 4），拖动其中任一个只改变宽度，拖动边的某个角点时保持屏幕宽度。`arc` 依次点击起点、终点与一个经过点；`curve` 依次点击起点、终点与其在 t = 1/2 处经过的点，`double_curve` 依次点击起点、终点与其在 t = 1/3 与 2/3 处经过的两点。存储的锚点仍是起点、经过点、终点（圆弧）以及贝塞尔起点、控制点、终点（曲线），`add_drawing` 接受同样的锚点；曲线的手柄位于曲线上（经过点），拖动它使曲线穿过指针而两端不动，拖动端点时曲线上的点保持不动。放置预览从最后一次点击之前起穿过指针，手柄圆盘画在已点击的位置上。
- 填充、延伸与端帽：`arc` 在 `fill_enabled` 时填充其与弦之间的圆弓形，`curve` 与 `double_curve` 填充曲线与其弦之间的区域（与弦相交的双曲线两瓣都填充）；这些区域与其他形状的填充一样只在绘图选中时是拖动面。`curve` 与 `double_curve` 按 `extend_left`/`extend_right` 沿端点切线延伸到窗格边缘。`arc`、`curve` 与 `double_curve` 按 `stroke_start`/`stroke_end` 在未延伸的端点绘制端帽，箭头可命中。
- 闭合折线：`tool_options.shape.closed` 把 `polyline` 的最后一个顶点连回第一个，外框为一条连续描边，不绘制端帽（端帽设置保留）；`fill_enabled` 时按非零规则填充封闭区域（超过 2,048 个顶点时只绘制外框）。新折线默认不填充，早期构建写出的闭合折线保持填充。
- 旋转矩形与三角形的外框是一条连续描边，角上没有缺口。

椭圆、圆、圆弧、曲线与双曲线在任意缩放下都与真实曲线相差不超过 0.25 设备像素，每条曲线的点数有界，窗格之外的部分几乎不产生工作量；虚线或点线外框与其他形状一样以实线虚线段到达每个执行器。`polyline` 像 `path` 一样放置顶点（点击添加，双击或 Enter 结束，放置三个或更多顶点后点击第一个顶点则闭合并结束，Backspace 或 Delete 删除最新的顶点，Escape 取消）。`highlighter` 与 `brush` 一样是自由手绘拖动，是一条半透明的 12 px 笔画，在自身重叠处每个像素只绘制一次（作为其覆盖的区域）。

### KLineChart overlay 的等价项

从 KLineChart 迁移的宿主可在此找到其每个绘图 overlay。其中七个是独立的工具（wire id 240 到 246）；其余则是带选项的现有工具，表格说明了具体做法。

| KLineChart overlay | Aeris 工具 |
|---|---|
| `straightLine` | `extended_line` |
| `rayLine` | `ray` |
| `horizontalSegment` | `horizontal_segment` |
| `verticalRayLine` | `vertical_ray` |
| `verticalSegment` | `vertical_segment` |
| `parallelStraightLine` | `parallel_channel`，设置 `fill_enabled: false` |
| `priceChannelLine` | `price_channel` |
| `fibonacciLine` | `fibonacci_retracement` |
| `priceLine` | `price_line` |
| `simpleTag` | `simple_tag` |
| `simpleAnnotation` | `simple_annotation` |
