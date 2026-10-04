# 通用图表 API：现状与提案

[文档导航](../README.md) · [API 入口](README.md) · [通用图表架构](../architecture/engine/general-charts.md)

本页保留已实现契约与后续设计；下文的支持范围说明优先于类型草案，尚未实现的系列不构成支持承诺。实际 React 导出见 [API 入口](README.md)，不要把早期阶段目标当作当前接入方式。

## 状态与目的

本文档最初是 [`plan.md`](../../plan/plan.md) 中一体化架构的阶段 0 API 提案，现仍是尚未完成的图表族的契约。当前的包已实现阶段 2 的笛卡尔坐标系图族：类别纵向柱形与水平条形、类别 band 箱线图、类别/类别以及数值/数值和时间/数值热力图网格、数值 XY 散点/气泡标记、数值/时间/类别误差棒，以及 `xy_line`、`xy_area`、`range_area`、分组/堆叠柱和堆叠面积切片。具备域感知能力的窗格、显式坐标轴、对象与类型化批量替换/更新、有界保留、行标签、快照、命中测试、无障碍和 V2 持久化，对这些已实现的种类/选项均已公开。`xy_line`、`xy_area` 和 `range_area` 支持连续数值、时间型 epoch 毫秒以及类别 band/point 的 X 域。`error_bar` 支持数值和时间型 X 坐标轴，两个坐标轴上均可带有可选的独立边界；并支持类别 band/point X，但仅可带可选的 Y 边界。`box_plot` 使用类别 band X 与数值 Y，并要求完整且有序的 `min <= q1 <= median <= q3 <= max` 行才能产生可见几何。所有形式都要求具备中心/取值通道，才会有可见标记。`heatmap_grid` 支持 band 的 X/Y 字符串类别、连续数值 X 搭配数值 Y 坐标，以及时间型 epoch 毫秒 X 搭配数值 Y 坐标。后续的极坐标系列名称在其实现和发布证据落地之前仍然只是提案。

通用系列的图例元数据由引擎拥有。`chart.general_legend_snapshot(pane?)` 返回有界的系列元数据，按稳定的引擎顺序排列，包含窗格、种类、标题、颜色与可见性。隐藏的系列仍然保留，并带有 `visible: false`，使宿主渲染的图例无需独立重建状态即可展示它们；窗格过滤是只读的，V2 恢复会重建出相同的语义条目。

跨系列的提示框分组同样由引擎拥有。`chart.general_shared_tooltip(series, row)` 返回同一窗格中可见的行，这些行的水平数据值与锚点精确匹配。顺序为稳定的系列顺序，其次是行顺序；重复的 X 值会被保留，隐藏的系列被排除，一个热力图对同一个 X 坐标/类别可以贡献多个单元格。

通用范围选择使用 `chart.set_general_brush(axis, from_coordinate, to_coordinate)`。浏览器仅发送一次 CSS 像素端点；Rust 会立即将其转换为数值型、时间型或类别型范围，并拥有这一临时选择。`general_brush_snapshot()` 返回该语义范围，以及所选坐标轴上可见行标识的有界列表。因此缩放/调整大小会对已存储的范围重新投影，而不是保留过期的像素。`clear_general_brush()` 将其移除。

参考组件是一等的引擎状态。`chart.add_general_reference(...)` 创建绑定到显式通用坐标轴的线、点或矩形区域。每个参考都声明 `extend_domain`；仅当该标志为 true 时，自动域才会包含其坐标。参考会转换为同一个与后端无关的帧，参与窗格/坐标轴的生命周期保护，并在 V2 持久化中往返还原。

本提案是增量式的。现有的金融系列、数据形状、窗格方法、价格比例尺句柄、snake-case 方法以及持久化 V1 保持其当前含义。特别是 `"line"`、`"area"`、`"bar"`、`"histogram"`、`"baseline"`、`"candlestick"` 和 `"footprint"` 仍然是金融时间系列。Aeris 绝不能猜测某一行属于金融数据域还是通用数据域。

命令式 API 仍然是规范的。当前包已经提供 camel-case 别名以及可选的 React 适配器，它们调用相同的变更，而不是另外定义一套图表模型；snake-case 方法继续受到支持。

## 领域词汇

首个实现为每个窗格绑定一个水平域。未来的绘制区域 API 可以允许在一个窗格内有多个区域，但必须保留这些相同的规则，且不得创建第二个引擎或帧。

```ts
type horizontal_domain_options =
  | { type: "financial_time" }
  | { type: "continuous"; scale?: "linear" | "log" | "symlog" }
  | { type: "temporal" }
  | { type: "category"; scale?: "band" | "point" }
  | { type: "polar" };

interface general_pane_options {
  preserve_empty?: boolean;
  horizontal_domain: horizontal_domain_options;
}
```

`financial_time` 仍然是初始窗格以及对 `chart.add_pane()` 或 `chart.add_pane(preserve_empty)` 的每一次现有调用的默认值。新增的重载为：

```ts
chart.add_pane(options: general_pane_options): pane_api;
```

引擎始终保留一个布局槽位。移除一个为空且被保留的最后窗格，会使该窗格的稳定标识退役，并在同一槽位中安装一个全新的、未保留的金融时间默认窗格；被移除的句柄变为过期。对有内容或未保留的最后窗格的移除会被拒绝。这使声明式所有者可以释放自己的窗格，而无需制造临时的占位窗格。

当窗格包含系列、坐标轴、选择或已持久化的通用数据集时，其水平域类型不可变。这避免了悄然重新解释已存储的坐标。空窗格可以在后续 API 中被显式重新绑定，但首个版本中先移除再重建即可满足需要。

数值型连续值是有限的 IEEE-754 数。时间型值是 `Date` 对象或有限的整数 epoch 毫秒数；它们会被一次性归一化为由引擎拥有的整数列。不接受 ISO 字符串和单位推断，因为它们的解析与单位不是确定的。现有金融数值时间仍为整数 UTC 秒，保持不变。

类别标签是 UTF-8 字符串，除非提供了显式域，否则由引擎按首次出现的顺序驻留。空字符串是有效标签。类别标识即精确的字符串；显示格式化器不会改变标识或顺序。

## 坐标轴词汇

通用坐标轴是显式的引擎对象。ID 是图表局部的、区分大小写的 UTF-8 字符串，长度为 1–128 字节。内置的金融价格/时间句柄仍然可用，并保留其当前的专用实现。

```ts
type axis_dimension = "x" | "y" | "angle" | "radius";
type axis_position = "top" | "bottom" | "left" | "right";
type general_scale_type =
  | "linear"
  | "log"
  | "symlog"
  | "temporal"
  | "band"
  | "point"
  | "radial_linear"
  | "angular_category";

type numeric_domain = "auto" | readonly [number, number];
type temporal_domain = "auto" | readonly [Date | number, Date | number];
type category_domain = "auto" | readonly string[];
type general_axis_tick =
  | { type: "numeric"; value: number; label?: string }
  | { type: "temporal"; value: Date | number; label?: string }
  | { type: "category"; value: string; label?: string };

interface general_axis_options {
  id: string;
  pane: number;
  dimension: axis_dimension;
  position?: axis_position;
  scale: general_scale_type;
  domain?: numeric_domain | temporal_domain | category_domain;
  reverse?: boolean;
  visible?: boolean;
  title?: string;
  tick_count?: number;
  ticks?: readonly general_axis_tick[];
  min_tick_gap?: number;
  band_padding_inner?: number;
  band_padding_outer?: number;
  zero_line?: boolean;
  grid_visible?: boolean;
}
```

校验是结构性且原子的：

- X 坐标轴必须与窗格的水平域匹配。
- Y 坐标轴可以是 linear、log 或 symlog；极坐标半径坐标轴使用 radial linear。
- log 坐标轴拒绝非正的显式边界，并在自动域贡献中忽略非正数据，同时将这些行保留为缺失几何。
- band 内边距为有限数，且仅当文档规定的范围允许时才会被钳制；无效值不会部分修改坐标轴。
- 自动域只合并绑定到该坐标轴的可见系列。参考组件显式声明是否扩展域。
- 在可执行的笛卡尔坐标轴上，`ticks` 以至多 512 个带类型的值取代自动刻度选择。这些值必须与坐标轴比例尺匹配且互不重复；数值型取值为有限数，对数取值为正数，时间型取值为 JavaScript 安全的 epoch 毫秒数。提供的标签作为可移植的引擎状态保留，并送达每个后端；省略标签时使用内置的数值、UTC 时间或类别格式化器。位于有效域之外的显式刻度会被裁剪。`tick_count` 与 `ticks` 互斥，因此被接受的选项绝不会同时带有两种相互竞争的选择策略。在极坐标刻度执行实现之前，极坐标显式刻度会被拒绝。
- 刻度放置、冲突剔除、网格贡献、标题和标签锚点由引擎拥有。宿主可以预先格式化显式刻度标签，但不能更改刻度坐标。
- `grid_visible` 在对应的图表级网格方向可见时，将该坐标轴的刻度坐标投影到被裁剪的窗格底层。`zero_line` 独立地在零位于数值域内时绘制一条实线。重合的线只输出一次，且零线优先，因此多个坐标轴不会加深共享坐标处的颜色。
- 时间型坐标轴选择一个有界的 UTC 间隔，范围从毫秒到日历年。日内、日、月、年标签使用注入的区域设置月份名称，并与数值和类别坐标轴共用同一个 `AxisFrame`；宿主不会运行并行的日期坐标轴布局。

首个实现应暴露 `chart.add_axis(options)`、`chart.axis(id)`、`chart.axes(pane?)` 和 `chart.remove_axis(id)`。移除非空的坐标轴会被拒绝。系列重新绑定是一个原子操作，当任一目标坐标轴不存在或不兼容时，操作失败且不产生任何变更。

## 通用系列名称

通用系列使用不会与现有金融含义冲突的名称：

```ts
type general_series_kind =
  | "xy_line"
  | "xy_area"
  | "range_area"
  | "range_bar"
  | "column"
  | "horizontal_bar"
  | "scatter"
  | "bubble"
  | "box_plot"
  | "heatmap_grid"
  | "error_bar"
  | "pie"
  | "donut"
  | "radar"
  | "radial_bar"
  | "polar_area";
```

分组和堆叠图表是兼容的纵向柱形、水平条形和面积系列上的选项，而不是独立的引擎或特定于渲染器的种类。仅当窗格、坐标轴、方向以及类别/连续坐标语义都匹配时，堆叠 ID 才会把系列连接在一起。

当前已实现的阶段 2 切片在 `column` 和 `horizontal_bar` 上暴露 `group_id`，并在两种柱方向以及 `xy_area` 上暴露 `stack_id`/`stack_mode`。纵向柱形将类别 X 坐标轴绑定到数值 Y 坐标轴；水平条形复用相同的类别/数值行，使用数值 X 坐标轴和 band Y 坐标轴。分组柱对类别 band 进行细分；匹配的堆叠柱占据一个分组槽位。堆叠面积图在数值、时间型和类别域中按精确的 X 标识对齐各成员，并在前一个累计边界与新的累计边界之间填充。`stack_mode: "normal"` 围绕零使用相互独立的正/负累计，而 `stack_mode: "percent"` 将正、负总量分别独立归一化为 `+1`/`-1`。两种柱方向都复用有序的 `Rect` 图元、精确的矩形命中、有界标签、快照、类型化/对象更新以及 V2 持久化。

`range_bar` 使用对齐的 low/high 数据集契约，搭配类别 X 坐标轴和数值 Y 坐标轴。每个完整的行转换为一个类别宽度的矩形，跨越其 low/high 值；缺失或无效的边界仍可查询，但不产生几何。范围柱使用精确的矩形命中，并与范围面积图使用相同的持久化与更新路径。

```ts
interface cartesian_series_options {
  pane?: number;
  x_axis_id: string;
  y_axis_id: string;
  visible?: boolean;
  title?: string;
  color?: string;
  /** Draw markers at line, area, or range-area data points; defaults to false. */
  point_markers?: boolean;
  /** Marker shape for scatter and path markers; defaults to circle. */
  point_symbol?: "circle" | "square" | "diamond" | "triangle";
  /** Stroke width for line, area, and range-area paths in CSS pixels; defaults to 2. */
  line_width?: number;
  /** Portable stroke pattern for line, area, and range-area paths; defaults to solid. */
  line_style?: "solid" | "dotted" | "dashed";
  /** Shared path interpolation for line, area, and range-area boundaries; defaults to linear. */
  interpolation?: "linear" | "step" | "curved";
  /** Bridge missing rows in path series; transform-invalid rows remain gaps. Defaults to false. */
  connect_missing?: boolean;
  /** Area fill opacity from 0 through 1; defaults to 72 / 255. */
  fill_opacity?: number;
  /** Explicit numeric fill baseline for `xy_area`; omitted uses zero when visible, otherwise the edge. */
  baseline_value?: number;
  group_id?: string;
  stack_id?: string;
  stack_mode?: "normal" | "percent";
  missing?: "gap" | "zero";
}

interface polar_series_options {
  pane?: number;
  angle_axis_id: string;
  radius_axis_id: string;
  visible?: boolean;
  title?: string;
  color?: string;
  stack_id?: string;
}
```

`missing: "zero"` 仅允许用于零是有意义基线的系列。线、散点、气泡、箱线图、误差棒和范围数据始终将缺失值视为不存在几何。

## 对象数据形状

对象行在包边界处一次性完成校验，并转换为带类型的引擎列。`null` 标记缺失的通道；`NaN` 和无穷值是无效输入，而不是另一种缺失哨兵值。

```ts
type general_row_id = string | number;
type general_x = number | Date | string;

interface xy_row {
  id?: general_row_id;
  x: general_x;
  y: number | null;
  color?: string;
  label?: string;
}

interface range_row {
  id?: general_row_id;
  x: general_x;
  low: number | null;
  high: number | null;
  color?: string;
  label?: string;
}

interface bubble_row extends xy_row {
  size: number | null;
}

interface heatmap_row {
  id?: general_row_id;
  x: string | number | Date;
  y: string | number;
  value: number | null;
  color?: string;
  label?: string;
}

interface error_bar_row extends xy_row {
  x: number | string | Date; // Number/Date for numeric or temporal X; string for band/point X.
  x_low?: number | Date | null;
  x_high?: number | Date | null;
  y_low?: number | null;
  y_high?: number | null;
}

// The numeric typed form has parallel x/y and x_low/x_high/y_low/y_high Float64Array columns.
// The temporal typed form uses x_epoch_ms plus x_low_epoch_ms/x_high_epoch_ms Float64Array columns;
// every present temporal X value and bound must be a whole JavaScript-safe epoch-millisecond integer.
// The category typed form has categories/category_indices, y, y_low, and y_high columns.
// Each bound has an optional Uint8Array validity mask (0 = absent), independent of y_valid.
// Present numeric/temporal X bounds must not cross X; with valid center Y, present Y bounds must not
// cross Y. Category rows reject X bounds. Absent bounds remain queryable.

interface box_plot_row {
  id?: general_row_id;
  x: general_x;
  min: number | null;
  q1: number | null;
  median: number | null;
  q3: number | null;
  max: number | null;
  color?: string;
  label?: string;
}

interface polar_value_row {
  id?: general_row_id;
  category: string;
  value: number | null;
  color?: string;
  label?: string;
}
```

缺少必需通道的行仍保留在数据集以及类别/域的并集中，但不产生标记。范围行要求 `low <= high`。箱线图行要求 `min <= q1 <= median <= q3 <= max`。负的气泡大小以及负的饼图/环形图取值是无效的。大小为零的气泡和取值为零的扇区会保留，用于查询和无障碍，但不产生可见面积。

如果省略 `id`，`set_data()` 会分配一个作用域限于该次安装批量的内部标识。对标识敏感的过渡以及按 ID 更新/移除的操作需要显式且唯一的 ID。重复的显式 ID 会使整个事务被拒绝。引擎绝不会把格式化后的标签或浮点哈希用作隐式标识。

## 类型化批量写入

批量 API 与对象形状一一对应，每个通道一列。它不接受行对象，不会对每个点调用回调，也不会对每个点跨越 WASM 边界。

```ts
interface numeric_xy_columns {
  ids?: readonly general_row_id[];
  x: Float64Array;
  y: Float64Array;
  y_valid?: Uint8Array;
  colors?: Uint32Array;
}

interface category_xy_columns {
  ids?: readonly general_row_id[];
  categories: readonly string[];
  category_indices: Uint32Array;
  y: Float64Array;
  y_valid?: Uint8Array;
  colors?: Uint32Array;
}

interface temporal_xy_columns extends Omit<numeric_xy_columns, "x"> {
  /** Whole epoch milliseconds, exactly representable as JavaScript numbers. */
  x_epoch_ms: Float64Array;
}

interface category_heatmap_columns {
  ids?: readonly general_row_id[];
  x_categories: readonly string[];
  x_category_indices: Uint32Array;
  y_categories: readonly string[];
  y_category_indices: Uint32Array;
  value: Float64Array;
  value_valid?: Uint8Array;
}

interface numeric_heatmap_columns {
  ids?: readonly general_row_id[];
  x: Float64Array;
  y_coordinate: Float64Array;
  value: Float64Array;
  value_valid?: Uint8Array;
}

interface temporal_heatmap_columns {
  ids?: readonly general_row_id[];
  /** Whole epoch milliseconds, exactly representable as JavaScript numbers. */
  x_epoch_ms: Float64Array;
  y_coordinate: Float64Array;
  value: Float64Array;
  value_valid?: Uint8Array;
}
```

所有并行数组的行数必须相等。有效性数组只包含 `0` 或 `1`。类别索引必须在范围内。在更改当前数据集之前，会先对事务进行校验。气泡在数值 XY 列的基础上增加 `size: Float64Array` 和可选的 `size_valid: Uint8Array`。箱线图使用类别索引，加上五个并行的 `Float64Array` 通道，名为 `min`、`q1`、`median`、`q3` 和 `max`，每个通道带有可选的有效性掩码。类别热力图使用相互独立的有界 X/Y 类别字典和对齐的索引列，外加一个数值 `value` 通道；在显式 ID 更新和有界保留期间，两个字典会原子地合并并压缩，而不是引入第二个数据存储。连续型与时间型热力图复用普通的数值/时间型 X 列，增加一个对齐的数值 `y_coordinate` 列，并将 `value` 保留在现有的取值/有效性通道中。单元格范围由相邻坐标中心确定性地推断得出。

首个版本需要 `set_data()`、`set_data_typed()`、按显式行 ID 追加/更新、有界保留、`data_at()`、命中测试/提示框快照以及容量遥测。通用存储在创建第一个通用系列或数据集时才惰性分配；仅含金融数据的图表必须保持零通用数据集、域、坐标轴和几何容量。

当前的柱形/水平条形/箱线图/热力图网格/散点/气泡/`xy_line`/`xy_area`/`range_area`/`error_bar` 浏览器切片暴露 `update_data(rows, { max_rows })` 和 `update_data_typed(columns, { max_rows })`。每个被更新的行都需要显式的字符串或数值 `id`；ID 匹配的行就地替换，新 ID 则按输入顺序追加。`max_rows` 是可选的、针对单次事务的保留上限：更新之后，会移除最旧的行，直到数据集不超过该上限。每次需要保留的流式更新都要传入它。无效的批量会使先前的数据集保持不变。被保留策略移除的行会丢失其标识以及任何悬停/选择目标；被保留的显式 ID 在头部裁剪之后仍继续标识相同的标记。

当前的柱形/散点/气泡/`xy_line`/`xy_area`/`range_area`/`error_bar` 选项还接受 `data_labels: true`，用于在标记附近绘制可见的数值 Y 值。引擎在共享帧中放置标签，拒绝重叠或位于绘制区域之外的放置，并将每个窗格每帧的输出上限设为 512 个标签和 4,096 次放置尝试。缺失的行绝不会产生标签。对象行可以提供 `label?: string`；类型化列可以提供并行的 `labels?: readonly (string | null)[]`。自定义文本会替换该行显示的数值，而空字符串则有意隐藏其可见标签。省略或为 null 的标签会回退为数值。替换或更新一行而不带自定义标签，会清除其旧文本；未触及的 ID 保留各自的文本。标签的校验、更新与保留和行事务一同原子完成。每个标签限制为 4,096 个 UTF-8 字节，每个数据集限制为 65,536 个自定义标签和 1,048,576 个标签字节。提示框与有界的无障碍快照会将自定义文本以 `label: string | null` 的形式与原始值一并暴露。

当前的图例接口按设计仅包含元数据：浏览器可以渲染 DOM 图例控件，但条目集合、顺序、可见性、标题、颜色、种类和窗格标识均来自引擎快照。坐标轴和系列句柄暴露 `set_visible`/`setVisible`；图例控件使用系列句柄，然后读取下一次引擎快照，而不是维护并行的可见性状态。

坐标轴和系列句柄还暴露 `apply_options`/`applyOptions`。坐标轴更新涵盖域、方向、位置、可见性、标题、刻度间距/数量、band 内边距、零线和网格策略；更改已配置的域只会重置该坐标轴的运行时视图。系列更新涵盖可见性、标题、颜色、点半径、数据标签、分组、堆叠标识、堆叠模式，以及兼容的窗格/坐标轴重新绑定。重新绑定要求目标窗格具有相同的水平域语义，并且目标 X/Y 坐标轴保持当前的比例尺类型。每次更新都会在提交前校验完整的候选状态，保留句柄和数据，并在被拒绝时保持先前状态不变。种类和数据集的更改仍然属于结构性变更。

数值型、时间型和类别坐标轴句柄暴露 `pan(fraction)`、`zoom(factor, anchor_value)` 以及 `reset_view()`/`resetView()`。时间型缩放锚点是 JavaScript 安全的整数 epoch 毫秒；类别锚点是其在当前可见窗口中的字符串标识。类别平移按可见类别数量的取整比例移动，并在基础域两端处钳制。类别视图保留的是有界的索引窗口，而不是复制的标签，因此自动域的变化不会在视口中遗留过期的类别标识。运行时视图会原子地拒绝无效锚点或会塌缩的变换，并保留已配置或自动的基础域以供重置。

`chart.general_series_order(pane?)` 按引擎稳定的自下而上顺序返回存活的句柄。`chart.set_general_series_order(handles, pane?)` 以原子方式执行，且仅接受该范围内所有存活通用系列的一个精确排列。窗格内的重排不会改变其他任何窗格的相对顺序。同一顺序驱动绘制、图例与命中测试遍历、React 带 key 的数组顺序，以及 V2 持久化。

参考组件有意与系列数据分离。参考线绑定一个 X 或 Y 坐标轴以及一个与之兼容的数值/时间/类别值；参考点绑定明确指定的 X 与 Y 坐标轴；参考区域在每个坐标轴上绑定两个端点。样式是有界的、由引擎拥有的状态。`extend_domain: false` 是默认语义：参考仅在其值映射到当前定义域内时才会绘制。`extend_domain: true` 会将每个已声明的参考坐标计入自动定义域解析。移除被存活参考使用的坐标轴会被拒绝，直至该参考被移除。

刷选状态是瞬态的，不会在 V2 中序列化。它与悬停/选择一样属于交互状态，而非图表配置。快照至多包含引擎有界的刷选项上限数量的条目，并保留系列/行顺序。共享提示框是派生快照，同样不会增加宿主侧保留的注册表。

## 窗格兼容矩阵

下文的 `same region` 表示这些系列可以共同贡献于同一个坐标区域和同一个有序帧。当比例尺类型能够接受这些系列的值时，该区域内允许使用不同的 Y 坐标轴。

| 现有或拟议的族 | 金融时间 | 连续 X | 时间型 X | 类别 X | 极坐标 |
| --- | --- | --- | --- | --- | --- |
| K 线、金融柱、直方图、基线、足迹图、指标 | 同一区域 | 不兼容 | 不兼容 | 不兼容 | 不兼容 |
| `xy_line`、`xy_area`、`range_area` | 不兼容 | 同一区域 | 同一区域 | 同一区域，使用点/带坐标轴 | 不兼容 |
| `column` | 不兼容 | 同一区域 | 同一区域 | 同一区域 | 不兼容 |
| `horizontal_bar`（数值 X，类别 Y） | 不兼容 | 同一区域 | 不兼容 | 不兼容 | 不兼容 |
| `scatter`、`bubble`、`error_bar` | 不兼容 | 同一区域 | 同一区域 | 同一区域，使用点坐标轴 | 不兼容 |
| `heatmap_grid`、`box_plot` | 不兼容 | 两个坐标轴一致时为同一区域 | 坐标轴一致时为同一区域 | 同一区域 | 不兼容 |
| 饼图、环形图、雷达图、径向条形图、极坐标面积图 | 不兼容 | 不兼容 | 不兼容 | 不兼容 | 同一极坐标区域 |

补充规则：

- 金融时间与时间型 X 绝不共享同一区域。金融时间是基于时间戳并集的逻辑索引间距；时间型 X 是按流逝时间的间距。外观相同的时间戳并不会使二者的语义兼容。
- 连续的线性、对数与 symlog 系列只有通过明确指定且相互兼容的 X 坐标轴才可共享窗格。允许使用多个 X 坐标轴，但每个系列仍使用该窗格的连续定义域类型。
- 带状坐标轴与点坐标轴可以共享同一个有序类别注册表。带状几何使用注册表区间；点几何使用其中心。明确指定的类别顺序必须相同，否则须使用不同的窗格/区域。
- 垂直柱与水平条仅当其 X/Y 角色互换后解析为同样的两个坐标轴注册表时，才可共享窗格。否则，类别 X 的垂直柱与类别 Y 的水平条不兼容。
- 堆叠系列要求相同的堆叠 ID、坐标轴 ID、类别键、方向和基线。分组系列要求相同的类别注册表，但可以使用彼此独立且兼容的 Y 坐标轴。
- 极坐标系列仅当其角度类别顺序与径向定义域语义一致时才可共享。
- 不兼容的添加操作会以 `invalid_options` 失败；Aeris 绝不会隐式移动系列，也不会创建隐藏的图表引擎。

## 每个通用系列必须共有的行为

在引擎拥有以下全部行为之前，新增公共系列种类就是不完整的：

- 输入校验与确定性的缺失值语义；
- 对自动定义域的贡献与对显式定义域的裁剪；
- 有序 `ChartFrame` 中与后端无关的几何；
- 精确命中与最近命中测试，并具有稳定的并列裁决；
- 提示框/值快照与格式化后的坐标轴值；
- 键盘焦点与无障碍快照值；
- 选择与悬停状态；
- 生命周期移除与有界的缓存失效；
- 在未来通用图表 schema 下的持久化；
- Canvas2D、WebGPU、GPUI 与原生的一致性证据；
- 仅金融、仅通用以及二者组合的性能证据。

宿主回调可以格式化标签或呈现 HTML 提示框，但内置几何、坐标轴放置、定义域计算、命中测试、提示框值与无障碍值仍归 Rust 所有。

## 首个垂直切片验收

类别柱形切片只有在证明了带状内边距、类别顺序、刻度碰撞、正/负基线、缺失行、标签、提示框/命中测试、无障碍以及全部执行器路径之后，才算就绪。XY 散点切片只有在证明了独立的连续 X 与 Y 定义域、在启用处的 log/symlog 校验、点大小边界、密集命中测试、平移/缩放、缺失行以及全部执行器路径之后，才算就绪。

首个 `xy_line` 切片只有在以下条件全部满足时才予以验收：连续数值、时间型与类别 X 定义域保持显式；缺失行会拆分描边段；已绑定的数据集不能更改 X 类型；线段命中、行标识、标签、无障碍、键盘导航、增量更新、V2 持久化以及 Canvas2D/WebGPU/GPUI 执行，全部使用同一套由引擎拥有的语义。

首个 `xy_area` 切片使用同一路径与标识契约，将每个连续段填充至由引擎拥有的基线，绝不跨越缺失或变换无效的行，并将填充区域纳入命中测试。共享帧先输出 `AreaFill`，再输出与之匹配的描边，并有 Canvas2D、WebGPU 与 GPUI 执行器的直接覆盖。

两个切片都必须与金融系列使用相同的图表生命周期、窗格句柄、主题、事件订阅、截图路径和有序帧。仅金融图表相对于 Phase 0 证据，其输出、稳态工作量、保留内存、启动或包体积都不得出现实质性变化。
