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
- 可选的顶层 `drawing_price_basis` 标签（宿主定义的绘图价格的价格基准）。

宿主的行情历史、系列与指标定义、图表选项、交易持仓/订单/成交/预览/意图、预警线/创建请求、自定义扩展、回调、订阅、选择、交互会话、代次、细节层级（LOD）、绘图边界/索引、保留的帧以及 GPU 资源均不持久化。宿主将 V1 恢复到一个全新的图表中，然后重新安装宿主拥有的数据、系列/指标配置、交易状态、预警状态、选项与扩展。

命名价格比例尺描述符与系列到比例尺的绑定同样由宿主拥有。宿主在恢复对比系列绑定之前，先在每个窗格中重建命名比例尺；图表状态 V1 保持不变。

导入会在变更之前检查整个文档，并作为一个事务安装。被导入的窗格获得全新的实时句柄 ID，同时保留各自独立的持久窗格 ID；因此导入前的窗格句柄与价格比例尺句柄会变为过期。仅当绘图 ID 尚未发放，且图表仍保持其初始窗格拓扑时，才接受导入。这可防止旧的绘图句柄重新指向具有相同持久 ID 的已恢复绘图。

绘图锚点以 `{logical, price, time?}` 的形式存储，另有可选的顶层 `drawing_price_basis` 标签；这两个字段均为可选，无需变更 schema 版本。在普通时间图表上，恢复时以 `time` 为准：在数据之后导入时，按每个锚点的时间在已加载窗口上解析锚点；在数据之前导入（网格工作区的顺序）时，锚点按时间保持待定状态，直到宿主安装数据。没有锚点时间的文档保留其逻辑锚点。非时间（tick/成交量/range）图表继续使用 `anchor_times_micros`。

绘图样式字段均为可选，省略时恢复该类型自身的默认值；导出仅在字段与默认值不同时才写入该字段。B8 绘图族选项存放在可选的 `style.tool_options` 对象中（序列化后至多 16 KiB），因此旧文档无需迁移。

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
