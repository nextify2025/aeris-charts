# 兼容性、错误与持久化

[文档导航](../README.md) · [架构总览](../Architecture.md) · [API 入口](README.md)

受支持的接口列表见[API 入口](README.md)。通用图表 V2 的数据与恢复契约见[通用图表](general-charts.md)。

- [实验性接口](#实验性接口)
- [错误与生命周期](#错误与生命周期)
- [数据写入与诊断](#数据写入与诊断)
- [持久化 V1](#持久化-v1)
- [持久化 V3 研究](#持久化-v3-研究)
- [版本策略](#版本策略)
- [品牌更名](#品牌更名)
- [发布策略](#发布策略)

## 实验性接口

自定义系列、窗格/系列/画布图元、导出的自定义系列/图元功能包、内置插件辅助函数、离屏 worker 图表、拆分网格辅助函数和快捷键辅助函数均为公共实验性 API。其当前的生命周期与隔离行为已有测试，但其确切类型可能在 1.0 之前的次版本发布中变化。`create_delta_tooltip()` 在 K 线系列上被有意设为不可用；K 线改用常规的悬停 `create_tooltip()`。delta 提示框仍可用于面积、折线、柱等非 K 线系列，并直接组合进可刷选面积交互中。可刷选面积是基于普通 `area` 系列的组合，而不是一种单独的承载数据的系列类型；旧的 `brushable_area` 输入写法仍作为兼容别名保留，并规范化为 `area`。该辅助函数在挂载期间，将鼠标/触控笔的主键窗格拖动保留给对比刷选；坐标轴拖动保持其普通的自动/手动比例尺行为，且该辅助函数不会全局禁用图表的滚动或比例尺选项。移除该辅助函数会立即恢复普通面积图的窗格拖动路径。常规的 `create_tooltip()` 是规范的结构化行情数据检视器。

其引擎快照为每种普通系列呈现方式（包括面积和折线）保留 Open/High/Low/Close；标量行自然在四个字段中报告相同的值，而被馈入保留的 OHLC 行的面积/折线系列可以渲染 Close，同时仍能检视完整的柱。宿主可以绑定显式的 `volume_series`，以添加与时间戳对齐的 Volume 行；图表绝不会猜测哪个直方图代表成交量。诸如 `create_rectangle_drawing()` 和 `create_rectangle_drawing_tool()` 之类的便捷辅助函数，是针对引擎持有的规范绘图类型的控制器；它们不定义单独的矩形功能或持久化标识。同样，视觉形状类似绘图的图元辅助函数仍然是图元，演示与宿主应当如此呈现。扩展在宿主渲染时运行，不得从渲染回调中重入图表变更，拥有其外部对象和持久化，并且恰好收到一次拆除通知。回调失败在宿主边界处被隔离，因此一个扩展不会妨碍其他扩展的拆除。任意扩展对象或可执行回调绝不会从持久化的 JSON 中重建。

## 错误与生命周期

可预期的失败会抛出 `AerisChartsError`，它是 `Error` 的子类，带有以下稳定错误码之一：`disposed`、`invalid_handle`、`stale_handle`、`invalid_data`、`invalid_options`、`unsupported_operation`、`serialization_error`、`persistence_version_error`、`extension_error`、`renderer_platform_error` 或 `resource_limit`。

`chart.remove()` 是幂等的。移除之后，每个需要有效图表状态的操作都会抛出 `disposed`。调用方已持有的标识字段仍然可以读取。已移除的系列、绘图、窗格和价格比例尺会抛出 `stale_handle`；过期的句柄绝不会指向替代对象。扩展清理异常仍被隔离，并作为开发警告报告。

创建具名价格比例尺时，会拒绝空的、保留的、重复的或过长的 ID、不存在的窗格，以及每个窗格的资源上限，且不产生部分变更。重新绑定到未知比例尺会抛出 `invalid_options`；移除内置比例尺或非空的比例尺会抛出 `unsupported_operation`。具名比例尺 ID 区分大小写，是窗格内局部的、长度为 1-128 字节的 UTF-8 字符串，每个窗格至多 16 个。

## 数据写入与诊断

干净的批量/当前柱写入保留其无分配/null 诊断路径。被修复、丢弃、重排、去重、拒绝或语义异常的输入，可通过 `series.last_ingestion_diagnostics()` 获取；OHLC 异常只报告，不改写数值。数值时间必须是有限的整数 UTC 秒，且位于闭区间 `-62167219200..253402300799`（年份 0000..9999）内，并且绝不自动转换。当缩放后会得到范围内的值时，拒绝原因会提示毫秒、微秒或纳秒。任何无效时间戳都会使直接的 set/update 批量被原子地拒绝；无效的单次 update 会使当前系列保持不变并在控制台发出警告，`update_typed` 亦如此。共享环形缓冲区的排空会逐行拒绝格式错误的行，并将其计入 `frame_stats().ring_dropped_rows`。worker 图表通过 `offscreen_chart.last_ingestion_diagnostics()` 暴露最近一次结果。

流式写入保持参考实现的 `series.update` 语义：一个数据点会替换其时间处的整根柱。`update()` 会对那些会静默改写柱的载荷报告机器可读的诊断 `code`，并指向 `merge()`：`value_on_ohlc_series`（`{ time, value }` 会压平 K 线/柱）、`price_less_payload`（例如 `{ time, volume }` 会变成空白数据），以及被拒绝的 `partial_ohlc`。对引擎拥有的系列（与成交绑定的 K 线或研究、重采样或合成的柱）的写入，在每条数据路径上都会以 `derived_series` 被拒绝；足迹图句柄则改为抛出 `unsupported_operation`。`series.merge(point, options?)` 是由引擎拥有的部分更新路径：存在的 open/high/low/close/value 字段会覆盖，缺失的字段保留现有柱的值，K 线/柱的结果会被规范化，使 `high >= max(open, close)` 且 `low <= min(open, close)`（针对新时间的仅含收盘价的 Tick 会创建 O=H=L=C；标量系列取 `value`）。

不含价格字段的合并会以 `empty_merge` 被拒绝；成交量和成交额会合并到各自的系列中。`series.merge_typed(columns, options?)` 是列式形式：第 `i` 行的合并方式与 `merge()` 相同，其中 `NaN` 条目和省略的列视为缺失；各行按输入顺序应用，仅触发一次引擎同步；只要有一行无效，整个批量即被拒绝（`offscreen_chart.merge_typed` 是 worker 形式）。自定义、高级和足迹图系列会抛出 `unsupported_operation`。流式写入的数据点若显式给出 `color`/`wick_color`/`border_color`，即使此前没有任何数据点带有颜色，也会为该柱着色，与 `set_data` 中同一项的行为完全一致。

`update`、`merge`、`update_typed` 和 `merge_typed`（包括 `offscreen_chart` 的类型化形式）接受 `{ sequence }`，其值为非负安全整数；无效值会被拒绝，并像无效数据一样发出警告。若提供了 sequence，且其不大于该系列上已应用的最后一个 sequence，则会被判为过期而拒绝（`code: "stale_sequence"`、`last_sequence`），不会改变数据，也不会触发 `data_changed`；不带 sequence 的调用始终会应用。完整的 `set_data`/`set_data_typed` 会清除该防护，或将其 `{ sequence }` 作为基线写入。该防护对每个系列为 O(1)，仅存在于运行时，不会持久化。自定义和高级系列在传入 sequence 时会抛出 `unsupported_operation`。Rust 宿主使用 `ChartEngine::merge_series_bar`、`merge_series_bars`、`update_series_bar_sequenced`、`update_series_bars_sanitized_sequenced` 和 `set_series_update_sequence`。

## 持久化 V1

持久化 schema 的版本控制独立于 npm 包版本。V1 仅包含：

- 有序的窗格标识、拉伸系数与空窗格保留标志；
- 有序的内置绘图，包含持久 ID、种类、窗格引用、语义锚点（`{logical, price, time?}`，在非时间柱图表上另加 `anchor_times_micros` 附带数据）以及样式；
- 可选的顶层 `drawing_price_basis` 标签（宿主定义的绘图价格的价格基准）；
- 顶层 `drawing_catalog` 标记（`2`，由每次导出写出，V2 与 V3 也包括在内），表明这些绘图遵循 AerisTerminal 上游的 B8 目录。
- 可选的顶层 `hidden_mark_groups` 列表（被隐藏的[时间线标记](presentation.md#时间线标记)分组，至多 64 个、每个至多 128 字节的 id；为空时省略，无需 schema 版本变更；V2 与 V3 文档携带同一个可选字段）。时间线标记本身从不持久化。

宿主的行情历史、系列与指标定义、图表选项、交易持仓/订单/成交/预览/意图、预警线/创建请求、自定义扩展、回调、订阅、选择、交互会话、代次、细节层级（LOD）、绘图边界/索引、保留的帧以及 GPU 资源均不持久化。（为带有通用窗格的图表写出的 V2 文档还携带图表选项存储——包括十字光标的 `vertLine`、`horzLine` 与 `shadeRight`；V3 文档与 V1 一样不携带图表选项。）宿主将 V1 恢复到一个全新的图表中，然后重新安装宿主拥有的数据、系列/指标配置、交易状态、预警状态、选项与扩展。

命名价格比例尺描述符与系列到比例尺的绑定同样由宿主拥有。宿主在恢复对比系列绑定之前，先在每个窗格中重建命名比例尺；图表状态 V1 保持不变。

导入会在变更之前检查整个文档，并作为一个事务安装。被导入的窗格获得全新的实时句柄 ID，同时保留各自独立的持久窗格 ID；因此导入前的窗格句柄与价格比例尺句柄会变为过期。仅当绘图 ID 尚未发放，且图表仍保持其初始窗格拓扑时，才接受导入。这可防止旧的绘图句柄重新指向具有相同持久 ID 的已恢复绘图。

绘图锚点以 `{logical, price, time?}` 的形式存储，另有可选的顶层 `drawing_price_basis` 标签；这两个字段均为可选，无需变更 schema 版本。在普通时间图表上，恢复时以 `time` 为准：在数据之后导入时，按每个锚点的时间在已加载窗口上解析锚点；在数据之前导入（网格工作区的顺序）时，锚点按时间保持待定状态，直到宿主安装数据。没有锚点时间的文档保留其逻辑锚点。非时间（tick/成交量/range）图表继续使用 `anchor_times_micros`。

绘图样式字段均为可选，省略时恢复该类型自身的默认值；导出仅在字段与默认值不同时才写入该字段（被清空的 `labels` 或 `levels` 列表写为 `[]`；`gann_fans`、`gann_arcs` 与 `level_*` 选项对其所属工具总是写出）。族选项块存放在可选的 `style.tool_options` 对象中（序列化后至多 16 KiB）；上游的工具将其选项作为普通样式字段持久化。

自有线早期构建写出的文档仍可加载。不含 `drawing_catalog` 的文档，若携带旧版种类名称或自有线自己的工具、锚点 `time`、`style.tool_options` 对象、`drawing_price_basis`、不含 `screen_x` 的 `anchored_text`，或者缺少上游为该工具每个绘图都会写出的某个字段的 B8 目录绘图（档位工具的 `level_*` 选项、`regression_deviations`、`icon_size`、`bars_pattern`、`wave_degree`），即被视为此类文档；最后这一条件用于识别在 Tick 柱、成交量柱或区间柱坐标轴上写出的此类文档（此类文档在这些坐标轴上不携带锚点 `time`）；来自上游固定修订的文档不携带上述任何一项，按原样加载。（自有线中不含标记的 Tick 柱、成交量柱或区间柱文档，若其 B8 绘图仅为圆弧、曲线、旋转矩形或正弦线等无档位的形状，同样不携带任何迹象，按未转换的形式加载。）恢复会在校验之前确定性地转换此类绘图：旧版种类名称映射到其规范种类；具有旧锚点数量的绘图获得上游的锚点（`disjoint_channel` 3 到 4、`gann_square_fixed` 1 到 2、`projection` 3 到 2、`price_note` 2 到 1、`signpost` 1 到 2、`bars_pattern` 2 到 3、`pattern_triangle` 4 到 5、`pattern_three_drives` 7 到 6）；在自有线的文档中，`arc`、`curve`、`double_curve`、`rotated_rectangle`、`fibonacci_speed_arcs`（原以第一个锚点为中心，现为第二个）、`fibonacci_circles`（原以两个锚点之间为中心，现以第二个锚点为中心）和 `sine_line`（原为两个相反的极值，现为一个零点交叉和一个极值）的锚点被转换为其新含义，`anchored_text` 以其窗格分数锚点作为屏幕位置，省略的值取写出它们的那个构建的默认值，具有扁平对应字段的 `tool_options` 键会移到该字段上（回撤或速度扇形的 `reverse` 会取反，因为自有线把它们的 0 档放在第二个锚点上；时间区与趋势时间的 `label_h_align` 互换 `left` 与 `right`；江恩扇形的 `reverse` 被丢弃，因为自有线的扇形从不读取它）。`flat_top_bottom` 依据第三个锚点的价格加载：不低于两个基线锚点时为 `flat_top_channel`，不高于两者时为 `flat_bottom_channel`，其水平线穿过基线时为 `disjoint_channel`，其第二条线就是该水平线。存储了 `gann.reverse` 的 `gann_square` 交换其两个锚点的价格，使上游的枢轴落在自有线的枢轴上；固定江恩方形的 `reverse` 已体现在其向下的对角中，二者的 `level_reverse` 都为 `false`；未设置 `scale_ratio` 的向下固定方形在加法偏移会越过零时把对角价格取为 `price / (1 + size_bars)`。转换得到的 `curve` 与 `double_curve` 控制点仅在所存储锚点的时间与其逻辑位置成仿射关系（锚点之间柱距均匀）且控制点位于这些锚点之间时获得时间标识，否则保留其逻辑位置；竖直轴 `rotated_rectangle` 的深度手柄位于所存储锚点之外，同样只保留其逻辑位置（跨越交易时段间隔的时间运算会把它们放到另一根柱上）。叉形线与叉形扇的每个档位 `v` 变为 `0.5 ∓ |v|/2` 两个档位并加上一个中线档位 0.5，可见档位按升序在前，下侧档位取其外侧档位的填充，因此线与填充区带与自有线一致。该构建从不写出的选项默认值会显式写入：六个线条工具的 `tool_options.line`，平行通道的 `middle_line`，回归趋势线的 `middle_line` 与 `show_pearsons`，斐波那契的 `trend_line`、速度扇形的 `grid` 与 `label_v_align`，江恩框的 `time_levels` 与江恩方形的 `show_stats`，标注（投影、便签、评论、价格便签、价格标签、路标、箭头标记与预测）的空 `tool_options.projection_annotation` 块；三角形形态的 `extend_left` 与 `extend_right` 为 `true`。回归线不对称或单侧的偏差保留在 `tool_options.channel` 中，`regression_deviations` 取已启用的较宽一侧。早期构建的剪贴板与同步载荷会获得锚点数量转换、柱形态取自其 `tool_options` 柱的快照，以及锚定文本取自其窗格分数锚点的屏幕位置；经过锚点数量转换的条目还会获得上述选项默认值（标注的空块、固定江恩方形的 `show_stats`、三角形形态的延伸开关），其固定江恩方形的 `reverse` 只保留在对角中，而 `labels` 恰为该构建五项默认统计且不带 `tool_options.line` 键的 `info_line` 获得该块（本构建在载荷中把缺失的块写为 `"line": null`，因此移除了统计框的已恢复信息线保持移除）；锚点数量不变的转换需要文档，因此锚点数量不变的载荷条目不获得上述选项默认值（平行通道与回归趋势线的 `middle_line`、回归的 `show_pearsons`，以及除上述信息线以外线条工具的 `tool_options.line` 外观），叉形线与叉形扇的条目保留自有线含义的 `levels`，不会转换为 `0.5 ∓ |v|/2` 加中线档位，因此其齿线落在另外的位置。部分转换会丢失细节：三驱动形态丢弃其最后一段，三角形形态多出 D→E 一段与 E 标签，投影丢弃其扇区半径，价格便签丢弃其通向标签框的引线与标签偏移（其第二个锚点），路标以零高度的杆开始，柱形态失去其框拟合，关闭了右侧延伸的射线与关闭了某侧延伸的延长线按射线与延长线加载，没有 `scale_ratio` 的固定江恩方形获得的第二个锚点可能远离该方形，叉形线的档位标签显示转换后的值，平顶与平底通道在反转的价格比例尺上把水平线画在基线锚点的价格上，旋转矩形可能在屏幕上沿其轴线滑动，速度弧朝其另一个锚点展开，而不是向上或向下，正弦波只从其零点交叉处起绘制，转换后的圆心位于价格中点（在对数比例尺上像素中点与之不同），反向螺旋线失去其逆时针旋转，回归的两侧偏差及其开关在其逐侧渲染重新实现之前，按取已启用较宽一侧的对称带绘制（+3/-1 的带绘制为 ±3，单侧带绘制为双侧），没有时间标识的转换后锚点（见上文）在恢复到另一历史窗口时不随其他锚点按时间重新解析。早期构建的剪贴板或同步载荷中的 `flat_top_bottom` 条目总是加载为 `flat_top_channel`，因此穿过或低于其基线的水平线在载荷中丢失（只有文档按第三个锚点分类）。早期构建保存的绘图模板不带出处，因此不获得该构建未写出的选项默认值：平行通道与回归趋势线的 `middle_line` 与回归的 `show_pearsons`，斐波那契的趋势线、网格与 `label_v_align`，江恩框的 `time_levels` 与江恩方形的 `show_stats`，线条工具的 `tool_options.line` 外观（统计框、角度线的参考线与圆弧、修剪的箭头），以及标注的自有线外观；这些绘图在重新设置这些选项之前保持上游的外观。叉形线与叉形扇的模板同样保留自有线含义的 `levels`，其齿线落在另外的位置。此外，对任何文档，便签、评论与标注框缺失的 `box_color` 与 `box_border_color` 恢复为空（导出器总会写出已设置的框颜色，因此缺失即表示已清除）。下一次导出会带着该标记写出转换后的绘图。转换的实现机制见[旧版工具名与文档迁移](../architecture/engine/drawing-families.md#旧版工具名与文档迁移)。

按维护者的决定，以下自有线渲染细节不予恢复，转换后的绘图与新绘图一样按上游的方式绘制：斐波那契速度扇形的时间射线及其延伸到窗格边缘，圆环、圆弧与楔形的档位标签位置，对数比例尺上的精确档位价格，以及区带按数值排序的顺序与 20% 的透明度；叉形线的 A–B 摆动虚线引导线、B–C 手柄连线与始终绘制的红色中线；江恩框四边的标签；矩形边界、方形与 45° 线段的拉直模式；回归通道区带 51 的透明度与逐侧拆分，以及回归偏差使用上游的总体标准差而不是自有线的样本标准差（`n − 1`）；形态的顶点标签保持上游的位置（顶点上方 8 px），不采用自有线带框、位于高点之上与低点之下的放置。

不受信任输入的限制为：每个文档 8 MiB、64 个窗格、10,000 个绘图、每个绘图 100,000 个锚点、总计 250,000 个锚点、每个绘图 64 KiB 文本，以及绘图文本总计 1 MiB。未知的可选 V1 字段会被忽略。未知的 schema 版本、绘图类型、窗格引用、重复 ID、无效的锚点数量、非有限/不安全的数字以及超出限制的情况，均会在结构层面失败，并使图表保持不变。V1 夹具是兼容性输入；未来版本必须为文档中记载的兼容窗口保留明确的 V1 迁移路径。

## 持久化 V3 研究

带有引擎持有指标的金融图表导出 schema 版本 3。V3 仍由宿主持有市场历史与普通系列数据，但会持久化有序的研究绑定、标量输入选择、类型化指标参数（包括显式的 seed、histogram 与 estimator 取值）、按时间戳对齐的成交量与成交额来源引用，以及各输出的样式。链式来源被编码为对较早研究输出的引用，因此恢复不依赖旧的实时系列标识。宿主必须在导入 V3 之前重新创建来源系列及其数据；导入会在变更之前校验每一项依赖、参数、输出样式数量和资源限制。V1 和 V2 文档仍被原样接受，且 V3 文档只能恢复到全新的金融图表中。

## 版本策略

浏览器包版本低于 1.0 期间：

- patch：兼容的正确性、安全、性能、文档与打包修复；
- minor：新增稳定 API、经明确评审的实验性 API 变更，或兼容的行为新增；
- major（包括最终的 1.0 边界）：移除/重命名/更改稳定 API 的签名、不兼容的稳定行为，或终止文档中记载的持久化兼容窗口。

新增绘图/系列类型通常属于 minor 级别的包功能，但改变既有类型的含义则不兼容。新增持久化 schema 不会使 V1 失效；移除 V1 支持遵循另行记载的持久化窗口，属于 major 级别的兼容性事件。

## 品牌更名

浏览器包为 `@aeristerminal/aeris-charts`（含 `@aeristerminal/aeris-charts/react`）。原先带有品牌名的错误导出已重命名为 `AerisChartsError` 和 `AerisChartsErrorCode`；迁移时请更新导入与 `instanceof` 检查。Rust 使用方使用仅限仓库的 `aeris_charts_*` crate。

每一个原先带有品牌名的公共标识符都已被彻底重命名：

| 公共接口面 | 新标识符 |
| --- | --- |
| 浏览器错误类 | `AerisChartsError` |
| 浏览器错误码类型 | `AerisChartsErrorCode` |
| React 图表组件 | `AerisChart` |
| React 图表 props | `AerisChartProps` |
| React 图表 hook | `useAerisChart` |
| WebAssembly 图表类 | `AerisChart` |
| WebAssembly 工作区类 | `AerisWorkspace` |
| GPUI 已准备帧类型 | `PreparedAerisFrame` |
| GPUI 视口类型 | `AerisViewport` |

持久化的图表与工作区 schema 标识符、浏览器事件名称、生成的 WebAssembly 资源名称、DOM ID/类名、CSS 选择器以及基准测试环境变量，现在均使用 `aeris_charts` 前缀。已停用的品牌没有任何兼容别名。

## 发布策略

标签发布依赖必需的 Rust、包以及可移植的 Chromium/Firefox/WebKit 作业。它还会检查持久化夹具、公共声明、Node/SSR 导入、包内容以及已配置的性能预算。可移植正确性禁止使用 `continue-on-error`。

对硬件和机器敏感的截图哈希、GPU 计时、堆采样和挂钟时间证据属于校准诊断。它们仍然不具权威性，不得仅为让某个 runner 通过而予以批准。共享绘制流一致性、裁剪/顺序/帧契约测试、回放确定性以及可移植的浏览器行为才是具有权威性的门禁。没有配置基准预算的场景仍明确为仅报告。
