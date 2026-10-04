# Rust 接入与固定修订升级

[文档导航](../README.md) · [架构总览](../Architecture.md) · [API 入口](README.md)

Rust crate 不发布到 crates.io。当前依赖与历史迁移说明分开阅读：迁移段落中的旧版本只适用于对应固定修订，不代表当前清单。

- [Rust 分发](#rust-分发)
- [更换固定修订](#更换固定修订)

## Rust 分发

Rust crate 仅限仓库内使用（`publish = false`）；不会向 crates.io 发布任何内容，浏览器包是唯一发布的产物。Aeris Terminal 等宿主通过固定的 Git 修订版本或本地路径使用 `aeris_charts_*` crate。Rust API 低于 1.0，可能在任何修订中变更，因此宿主在更换其固定修订时应查阅下面的说明。

`aeris_charts_render_gpui` 是实验性的。它以精确版本要求固定 `gpui-pre` 0.3.7，即 gpui-kit 0.7.0 所依赖的 GPUI 快照，因此绘制图表的宿主必须使用同一个 `gpui`（宿主若使用其他 GPUI 构建，例如 Zed 的某个 Git 修订版本，就会持有其类型的两份不兼容副本）。GPUI 升级是显式的 manifest 与 lockfile 变更。

在 macOS 上，宿主必须启用 `font-kit` feature 来构建其 GPUI 平台 crate（`gpui-pre-platform`，或直接构建 `gpui-pre-macos`），该 feature 即 GPUI 的 macOS 文本系统。否则 GPUI 会改用空操作的文本系统：图表不绘制任何文本（坐标轴、标签、图例、绘图文本），并将每个字符串的宽度度量为零，唯一的信号是启动时的一条 `log::warn!`。Linux 和 Windows 不受影响。

通过 `GpuiChartInput::scroll_wheel` 绑定滚轮事件的宿主可获得与浏览器等效的滚动。固定在较早修订的宿主在水平滚轮或触控板横扫时，平移时间比例尺的方向相反：适配器未翻转就透传了 GPUI 的水平增量，而 GPUI 报告的是内容运动（正值表示露出左侧），引擎则与浏览器一样，将正值视为向右移动。适配器现在会翻转水平轴；自行补偿了旧符号的宿主必须移除其补偿。

## 更换固定修订

以下说明列出了更换固定修订所带来的宿主可见变更，以便调用点只需评审一次。每个分组都会指明修订级别的变更及其影响的调用点。

每个分组都以承载其变更的提交为键。分组适用于不包含该提交的固定修订（`git merge-base --is-ancestor <commit> <pin>` 以非零状态退出）；已包含该提交的固定修订已经采纳了该变更。哈希为本仓库的哈希。“上游”标记来自 `AerisTerminal/aeris-charts` main 并在此保留其哈希的提交，因为上游是通过合并引入的，从未变基；“自有线”标记仅存在于本仓库的提交。两条历史在 `ed2910d` 处分叉，旧写法仅存在于某一侧的分组会注明是哪一侧。

**子窗格坐标**（自有线，来自 `84b85e6 fix(engine): sub-pane crosshair sync, chart-level pane selection, coordinate contract`；参见[坐标与窗格](coordinates-and-time.md#坐标与窗格)）。三项行为发生了变更：

- 图表级的 `price_to_coordinate` 和 `coordinate_to_price` 不再跟随按创建顺序排列的第一个可见系列。价格在窗格 0 的默认比例尺上换算，坐标则在包含 `y` 的窗格的默认比例尺上换算，因此依赖于叠加系列先创建、或主系列被隐藏的调用必须改用该系列自己的句柄。主系列最先创建的单窗格图表不受影响，系列句柄的换算从未改变。
- 下方窗格（`pane_index` 为 1 或更大）上的联动十字光标现在可以往返一致。较早的修订将窗格偏移量应用了两次并读取了错误的比例尺，因此 `crosshair_sync_position` 与 `apply_external_crosshair` 在第一个窗格之外的任何窗格上结果都不一致。同步的价格现在是窗格默认比例尺上的价格，超出范围的价格会把该线保持在其窗格内。
- `ChartEngine::pane_index_at_y`（以及浏览器包中的 `pane_index_at_y`）现在对分隔条返回其上方的窗格，对位于内容上方的 `y` 返回窗格 0，而这两种情况过去都解析为最后一个窗格。为价格坐标轴命中测试选取窗格的 GPUI 宿主会获得修正后的映射。

**成交流派生系列**（自有线，来自 `a51242c fix(engine): guard trade-stream-derived series against host writes`；参见 [Tick 转 K 线](aggregation.md#tick-转-k-线)）。需要评审两项，其中第一项无法在本仓库中检查：

- Terminal 不得向通过 `bind_trade_bar_series_to_stream` 绑定的 K 线，或向 CVD、delta 或成交量系列（`add_cvd_series`、`add_delta_series`、`add_trade_volume_series`）写入。宿主对它们的每一次数据写入现在都会像对足迹图的写入那样被拒绝（`false`、`0`、`None`、`Err(UnsupportedSeriesData)` 或 `Rejected(UnsupportedSeries)`；`series_is_source_owned(id)` 可将该拒绝与未知 id 区分开），且 `apply_momentum_histogram_colors` 对 delta 与成交量研究返回 `false`。浏览器包以 `code: "derived_series"` 拒绝这些写入。
- `FootprintError` 新增 `SeriesOwned(SeriesId)` 变体，因此对它做穷尽 `match` 的代码需要补一个分支。对于已被重采样器、合成柱或某个被转换为 K 线的研究写入的 K 线或柱系列，`bind_trade_bar_series_to_stream` 会返回它（足迹图或标量研究仍得到 `UnsupportedTradeBarSeries`，因为 K 线类型检查先运行）；对于已被成交流、研究、重采样器或合成柱写入的系列，`configure_footprint_series` 会返回它。重采样目标和合成柱系列会拒绝已绑定成交流的 K 线（`ResampleError::UnsupportedTarget`、`SyntheticBarError::UnsupportedSeries`）。

**批量化的周期重置研究线**（自有线，来自 `fafecda perf(render): batch period-reset study segments into one Segments primitive`）。这对穷尽匹配 `aeris_charts_render::draw_list::Prim` 的 Rust 代码（自定义执行器、帧检查器、点池重新定基）构成编译期破坏性变更：

- `Prim` 新增 `Segments { first_point, segment_count, width, color }`，它是对 `points[first_point .. first_point + 2 * segment_count]` 的 `segment_count` 条相互独立的两点描边的批量，每条的描边方式与实线的简单两点 `Polyline` 相同（虚线已展开为每段虚线一对点）。对于日线或更长周期柱上的交易时段 VWAP、VWAP 带和枢轴线，引擎会发出它来取代每根柱一条的两点 `Polyline`，因此能通过编译的 `_ => {}` 分支会悄悄停止绘制这些研究。请从 `draw_list::segment_points` 获取点对窗口（超出点池的范围会使该图元被丢弃），并在重新定基某一层时，将 `first_point` 与其他所有点池索引一并移动。本仓库中的 Canvas2D、WebGPU、GPUI 和原生执行器已经处理了它。由于宿主通过更换固定修订来采纳该变更，因此对穷尽匹配的代码而言它是破坏性的。

**命名时区**（两条线，来自合并 `2e7d19f merge: sync with AerisTerminal/aeris-charts main`，该合并将上游的 `f796529 feat(time): add selectable IANA chart time zones` 引入自有线的交易所时间时钟；参见[时间、交易所时区与交易时段](coordinates-and-time.md#时间交易所时区与交易时段)）。需要评审的行为：

- `ChartEngine::set_time_zone` 接受来自 `TRADINGVIEW_TIME_ZONES` 的 IANA id（`Result<bool, String>`；已安装时返回 `Ok(false)`）。接受 `UtcOffsetSchedule` 的设置方法是 `set_exchange_offsets`，因此针对 `set_time_zone` 接受 schedule 的修订所编写的调用点必须改用新名称。只有合并之前的自有线固定修订接受 schedule；从 `f796529` 起的上游固定修订已经传入 id。
- 从 `f796529` 起的上游固定修订在 `aeris_charts_core` 中带有感知时区的辅助函数，每个都接受 `ChartTimeZone`：`format_tick_label_with_time_zone`、`format_date_pattern_with_time_zone`、`format_crosshair_time_with_time_zone`、`weight_by_time_in_time_zone` 和 `fill_weights_for_points_in_time_zone`。该合并删除了它们，并保留自有线的 `ExchangeTime` 形式作为唯一的时钟（自有线固定修订已具备这些形式）；它们接受 `&ExchangeTime` 而非时区：`format_tick_label_in`、`format_crosshair_time_in`、`weight_by_time_in` 和 `fill_weights_for_points_in`。用 `ChartEngine::exchange_time()` 读取引擎自己的时钟，或用 `ExchangeTime::new(zone.offset_schedule()?, 0)?` 构建一个。`format_date_pattern` 没有 `ExchangeTime` 形式：请用 `ExchangeTime::local_seconds` 平移时间戳并对结果格式化，`format_crosshair_time_in` 即是如此。只有从 `f796529` 起的上游固定修订具有被删除的辅助函数；自有线固定修订从未具有。
- 命名时区会一次性解析为显式 schedule，因此刻度权重、标签、VWAP 与枢轴的周期键、交易时段和倒计时都与 `set_exchange_offsets` 完全一样地遵循它。该名称还会本地化通用时间轴和 `time_zone_clock_text`；显式 schedule 则不会，此时 `time_zone_id()` 返回 `custom`，而不是 TradingView id。
- 顶层 `timezone` 选项若不是字符串，或其 id 不在一致性列表之内，现在会拒绝整个选项补丁（较早的修订会静默忽略它）。转发 TradingView 占位符（例如 `exchange`）的宿主必须在打补丁之前将其过滤掉。导入已保存的 V2 文档是例外：其选项中无法解析或非字符串的 `timezone`（较早的构建所存储的原始值）会被丢弃，以便布局的其余部分仍能恢复。
- `time_scale_options_json()["time_zone"]` 报告交易所 schedule（`"UTC"` 或转换数组），而不是较早修订在此处输出的 TradingView id；请通过 `time_zone_id()` 读取命名时区。
- V2 文档可以在 `timeScale.timeZone` 旁携带一个新增的 `timezone` 字符串，仅在安装了命名时区时写入。通过 Git 依赖 `aeris_charts_core` 的使用方不会读取本仓库的 `.cargo/config.toml`，因此它会编译完整的 tz 表，而不是本仓库产物所保留的 98 个一致性时区。

**绘图文本编辑**（两条线，来自合并 `2e7d19f merge: sync with AerisTerminal/aeris-charts main`，该合并使上游的 `7518e7e feat(drawings): engine-owned text typing session for every host` 成为唯一的会话，并保留了自有线的布局、命中测试和可编辑工具；参见[绘图锚点、磁吸与价格基准](drawings.md#绘图锚点磁吸与价格基准)）。每一项都会说明它适用于哪些固定修订：

- 在 main 上，一个引擎会话是唯一的文本编辑状态。用 `begin_drawing_text_edit(id, paint_caret)` 打开它（自行绘制插入符的宿主传 `false`，浏览器即如此）；用 `set_drawing_text_edit(text, caret)` 镜像宿主的可编辑表面；用 `commit_drawing_text_edit()` 或 `cancel_drawing_text_edit()` 结束它；`editing_drawing()` 读取已打开的会话。原生宿主使用 `drawing_text_edit_insert`、`drawing_text_edit_key`、`drawing_text_edit_select_all` 和 `drawing_text_edit_caret_at`。实时文本不记录撤销步骤；提交会记录一步。将键盘事件转发给 `GpuiChartInput::key_down` 的 GPUI 宿主无需调用这些方法：适配器会路由按键，包括平台的按词和按行移动以及剪贴板快捷键（参见下文的引擎输入控制器）。
- `ChartEngine::set_editing_drawing` 已被移除。上游固定修订具有它（与该会话并存），`36c9f09` 之前的自有线固定修订也具有；从 `36c9f09` 起的自有线固定修订则没有。
- 从 `36c9f09 feat(charts): B8 drawing catalog, multi-calendar overlays, bounded ticks, tick-built candles, and resampling` 起直到该合并为止的自有线固定修订使用其自己的三调用会话，该合并已将其移除：`begin_drawing_text_edit(id)`、`set_drawing_edit_text(text)` 和 `end_drawing_text_edit(commit)`。新的调用为 `begin_drawing_text_edit(id, paint_caret)`、`set_drawing_text_edit(text, caret)`（镜像值现在携带插入符），以及分别对应 `commit` 为 true 或 false 的 `commit_drawing_text_edit()` 或 `cancel_drawing_text_edit()`。这不是单纯的重命名：旧调用既不修剪文本，也不移除文本工具，而现在提交会修剪文本并移除留空的文本工具，取消则会移除一开始就为空的文本工具。上游固定修订从未有过三调用形式。
- 包含 `7518e7e` 但不包含 `b75f092 feat(drawings): text-field selection in the drawing typing session` 的上游固定修订具有 `drawing_text_edit_key(key)`。该提交将其改为 `drawing_text_edit_key(key, extend_selection)`（Shift 扩展选择），并为 `DrawingTextEditKey` 增加了变体 `DeleteWordBackward`、`DeleteWordForward`、`WordLeft` 和 `WordRight`，因此这样的固定修订需要加上该参数，并且对按键做穷尽 `match` 的代码还需补上这些分支。自有线固定修订从未有过单参数形式。
- `begin_drawing_text_edit` 接受每一种自行绘制文本的绘图，并拒绝（不影响已打开的会话）已锁定、已隐藏、按周期隐藏或非文本的绘图，以及锚点尚无法换算的绘图。上游的会话只打开文本工具和趋势线，并且即使拒绝也会关闭已打开的会话；`d438dab` 之前的自有线固定修订止步于这些以及绘图族中的绘图。尚未布局的图表没有可用于换算锚点的价格比例尺，因此在第一帧之前就开始会话的宿主会得到 `false`。
- 上游固定修订将文本限制在 256 字节；main 以 `MAX_DRAWING_TEXT_BYTES`（65,536 字节）为界：会超出该界限的插入整体被拒绝，镜像值则在字符边界处被钳制。文本 run 标签保持在一行内；绘图族文本框（`comment`、`callout`、`note`、`signpost`、`anchored_text`）保留换行。原生宿主目前支持点击定位插入符和在文本框中输入，但尚不支持上下行导航。
- `drawing_text_hit_at` 会对每一种绘制文本 run 的工具（线条、通道、斐波那契、形状）的标签作答，而不仅限于趋势线，并会与更上层的绘图主体进行仲裁；上游固定修订和 `d438dab` 之前的自有线固定修订只对趋势线作答。哪一次点击开始输入，对所有宿主是同一条规则：第一次点击仅对文本工具的两步点击和趋势线标签打开输入，而放置会请求编辑器的工具时则在放置时打开；对已选中绘图的双击、Enter 或 F2 会打开每个带文本的绘图的编辑器。浏览器手势层和原生输入控制器都应用该规则；直接驱动引擎 API 的宿主需自行应用。
- 上游的 `3527136 Default rectangle borders off and EMA strokes to one pixel`（合并 `2e7d19f` 之前的自有线固定修订和 `3527136` 之前的上游固定修订都不具备它）使矩形默认无边框（`border_visible: false`）。省略该键的已保存文档导入时边框可见，因此较早的文档保持其外观。

**引擎输入控制器**（上游，来自 `17a591f feat(input): engine-owned interaction controller for every native host`，自有线通过合并 `3eef45e` 引入；设计见[共享输入控制器](../architecture/engine/input.md)）。引擎现在拥有指针、滚轮和按键的路由：宿主将平台事件转换为 `PointerInput`、`WheelSample` 和 `ChartKey`，并调用 `ChartEngine::input_*`。按下仲裁、拖动生命周期、点击与双击、键盘绑定、悬停、光标选择和惯性运动都归引擎所有。请评审以下调用点：

- 哪些固定修订具有已移除的手势 API。`begin_financial_drag`、`update_financial_drag`、`update_financial_crosshair`、`end_financial_drag`、`apply_financial_wheel`、`apply_financial_navigation`、`financial_drag` 以及类型 `FinancialDrag` 和 `FinancialNavigation` 仅存在于上游一侧，来自 `f9b052f refactor(engine): own native financial gestures`（`apply_financial_navigation` 和 `FinancialNavigation` 来自 `4aaaf78 refactor(engine): own financial scale commands`），并存在于自有线从合并 `2e7d19f` 起、直到合并 `3eef45e` 引入 `17a591f` 为止的修订上，该提交移除了它们，且没有提供兼容层。`2e7d19f` 之前的自有线固定修订从未有过它们，因此没有需要删除的宿主手势 API：它直接采用 `input_*` 调用，下表只说明每个调用现在的行为。坐标未变（窗格空间：x 自 plot 区域左边缘起，y 自图表顶部起）。

  | 已移除 | 现在 |
  | --- | --- |
  | `begin_financial_drag(x, y, click_count, radius)` | `input_pointer_down(PointerInput, click_count)`；`click_count` 为 `u32`（原为 `usize`）；4 px 的分隔条半径是常量 `PANE_SEPARATOR_HIT`；双击坐标轴重置遵循 `InteractionOptions::axis_double_click_reset_time` 和 `axis_double_click_reset_price` |
  | `update_financial_drag(x, y)`、`update_financial_crosshair(x, y, radius)` | `input_pointer_move(PointerInput, primary_pressed)`，它还会对绘图、系列和交易执行悬停提升，并在按下被捕获期间保持十字光标跟踪（限制在 plot 区域内）；`input_pointer_leave()` 在没有处于打开状态的按下时清除十字光标 |
  | `end_financial_drag()` | `input_pointer_up(PointerInput)` 用于完成，`input_cancel()` 用于放弃 |
  | `apply_financial_wheel(x, y, normalized_x, normalized_y)` | `input_wheel(WheelSample)`，返回图表是否消费了该滚轮事件。在默认的 `WheelBehavior::Auto` 下，`delta_y` 像 `normalized_y` 那样经过 `wheel_zoom_scale`，`delta_x` 像 `normalized_x` 那样经过 `WHEEL_SCROLL_PX_PER_DELTA`，因此作为 `normalized_x` 和 `normalized_y` 传入的值原样传给 `delta_x` 和 `delta_y`。它在行为上并不等价：参见下文的差异 |
  | `apply_financial_navigation(action, accelerated)` 和 `FinancialNavigation` | `input_key_down(ChartKey, InputModifiers, repeat, now_ms)` 和 `input_key_up(ChartKey)`：`PageUp`/`PageDown` 对应 `PreviousPage`/`NextPage`，`ZoomIn`/`ZoomOut` 保持原名，`accelerated` 即 `InputModifiers` 中的 Control 或 Shift。`PreviousBar`/`NextBar` 仅在方向上映射到 `ChartKey::ArrowLeft`/`ArrowRight`：参见下文的差异 |
  | `financial_drag()` 和 `FinancialDrag` | 无：已打开的手势是引擎私有的，指针反馈为 `input_cursor()` |

- 替代项与其所替代的调用行为不一致之处。这些是在代码中比对的，而非仅比对签名，且被移除的函数自引入起到 `17a591f` 之间都没有变化：
  - 价格坐标轴上的滚轮。无论滚轮行为如何，只要指针位于某个价格比例尺的坐标轴条带上，`apply_financial_wheel` 就会缩放该价格比例尺。`input_wheel` 仅在 `InteractionOptions::wheel_behavior` 为 `WheelBehavior::Zoom` 或 `InteractionOptions::price_axis_wheel_zoom` 为 `true`（默认 `false`）时才缩放它；在默认的 `WheelBehavior::Auto` 下，同样的滚轮在坐标轴条带上缩放的是时间比例尺。要恢复旧行为，请以 `price_axis_wheel_zoom: true` 调用 `set_interaction_options`。`WheelBehavior::Zoom` 在那里同样缩放价格比例尺，但它会把每一次滚轮都变成缩放，因此水平滚轮不再平移。
  - plot 区域上的滚轮缩放。旧调用围绕指针缩放时间比例尺。普通滚轮现在遵循 `right_bar_stays_on_scroll`（默认 `true`；参见下文的后续变更），因此最新柱之后的间隙得以保留，只有 Ctrl/Cmd 才会围绕指针缩放。`input_wheel` 还遵循 `wheel_zoom` 和 `wheel_scroll`（两者默认均为 `true`），并会停止正在进行的惯性滑行、按住方向键的平移或动画滚动。
  - 拖动阈值。旧调用从第一次 `update_financial_drag` 起就生效（窗格平移、坐标轴缩放或分隔条调整大小）。现在窗格平移、坐标轴缩放和分隔条拖动都在共享的 5 px 阈值处开始，未移动的释放是一次点击。因此在价格坐标轴上的普通按下不再将该比例尺切换为手动：旧的 `begin_financial_drag` 在按下时就关闭自动缩放，现在则由第一个缩放步骤来关闭。窗格拖动仅在手动价格比例尺是按下位置下方系列（或窗格默认）的比例尺时才平移它；旧调用还会回退到窗格的第一个手动右侧或左侧比例尺。
  - 价格坐标轴上的双击。`begin_financial_drag` 会运行 `reset_price_scales()`，即图表中的每个价格比例尺；`input_pointer_down` 只重置被按下的比例尺（`reset_price_scale(pane, target)`）。需要图表范围重置的宿主应为其自己的命令保留 `reset_price_scales()` 或 `reset_view()`。
  - 释放与取消。`end_financial_drag` 只结束缩放和滚动会话。未移动的 `input_pointer_up` 还会选择或激活指针下方的对象，`input_cancel()` 结束同样的会话，但还会还原已打开的绘图或交易拖动，并清除悬停、实时测量和光标。在按下处于打开状态时，`primary_pressed` 为 false 的 `input_pointer_move` 会放弃该次按下。
  - 方向键与缩放键。旧调用每次调用跳转 1 根柱（加速时 10 根）；`ArrowLeft` 和 `ArrowRight` 启动由速度持有的平移（参见关于时钟的一项）。`ZoomIn` 和 `ZoomOut` 锚定在 plot 区域中心；在新的 `right_bar_stays_on_scroll` 默认值下，它们改为保留最新柱之后的间隙。按键还遵循 `InteractionOptions` 的开关（滚动键使用 `pan` 或 `wheel_scroll`，缩放键使用 `wheel_zoom`；默认均为 `true`），被开关拦住的按键保持未被消费。
- 宿主提供时钟。方向键平移由速度持有，因此需要调用 `input_key_up`，传入平台的按键重复标志，每准备一帧调用一次 `input_tick(now_ms)`，并且仅在 `input_animating()` 成立期间请求下一帧。`input_wake_deadline_ms()` 是唯一的延迟截止时间（交易提示框的停留时间）：为它安排一次唤醒并重绘。`flush_coalesced_input()` 每次 prepaint 转发一次最新捕获的绘图采样。
- 引擎将仅宿主可做的工作以 `ChartInputEvent` 事件的形式交还（`ContextMenu`、`DrawingCreated`、`RemoveSeries`）；每次输入调用之后用 `take_input_events()` 取出它们。宿主保留事件转换、指针捕获、应用 `input_cursor()`、定时器与帧调度、菜单、剪贴板和持久化。应在 `drawing_revision()` 变化时持久化绘图，而不是跟踪手势。参考实现的 `handleScroll`/`handleScale` 开关即 `InteractionOptions`（`interaction_options()` 和 `set_interaction_options`）。
- 较低层的手势操作（`drawing_tool_pointer_*`、`measure_pointer_*`、`delta_tooltip_mouse_*`、`kinetic_*`、`start_keyboard_scroll`、`keyboard_scroll_tick`、`cancel_keyboard_scroll`、`time_axis_*` 和 `price_axis_*` 的缩放与滚动步骤、`drag_pane_separator`）仍然是公开的引擎操作，但现在由控制器对它们排序。仍在 `input_*` 之外继续驱动它们的宿主会绕过控制器的按下仲裁和光标，因此对它们排序的宿主路由应当删除。
- 帧准备随之变化：`prepare_financial_frame_with_measure` 在任何图层失效或输入变化之后重建，并在窗格调整大小的拖动之后重新布局。曾清空其帧以强制重建、或在 `update_financial_drag` 报告窗格调整大小时强制布局的宿主，可以停止这样做。
- GPUI 宿主（feature `gpui-backend`）通过 `aeris_charts_render_gpui::input` 绑定，每个监听器一次适配器调用：`GpuiChartInput::mouse_down`、`mouse_move`、`mouse_up`（图表之外的释放也要绑定）、`context_menu`、`scroll_wheel`、`pinch`、`modifiers_changed`、`key_down` 和 `key_up`。`scroll_wheel` 自行转换 GPUI 的增量（像素除以 100，行按 `WHEEL_LINE_HEIGHT`（32 px）计）。prepaint 调用 `set_canvas_bounds(bounds)` 和 `prepare_frame(&mut engine)`，`wake_delay(&engine)` 安排延迟唤醒，`cursor_style(engine.input_cursor())` 是唯一的光标映射，`install_text_metrics(&mut engine, window)` 在准备帧之前运行，使绘图标签的度量与其绘制一致。宿主自己的按键表、光标优先级和文本编辑路由在适配器之外是多余的，应当删除而不是保留。
- `17a591f` 之后的两项后续变更。`1869773 feat(input): TradingView wheel zoom anchoring, engine-owned on every host`（上游）将 `GpuiChartInput::set_origin(Point<Pixels>)` 重命名为 `set_canvas_bounds(Bounds<Pixels>)`（只有恰好位于 `17a591f` 的固定修订具有旧名称），新增了带有 `GpuiChartInput::pinch` 的 `input_pinch`，并使 `TimeScaleOptions::right_bar_stays_on_scroll` 默认为 `true`：普通滚轮或键盘缩放会保留最新柱之后的间隙，只有 Ctrl/Cmd 滚轮和捏合缩放才围绕指针缩放。如需较早的围绕指针（滚轮）或 plot 区域中心（按键）缩放，请调用 `ChartEngine::set_right_bar_stays_on_scroll(false)`。合并 `3eef45e merge: sync with AerisTerminal/aeris-charts main (range tools, input controller)`（自有线）新增了 `ChartKey::EditText`（F2），因此上游固定修订上对 `ChartKey` 的穷尽 `match` 需要补一个分支。

**GPUI 依赖**（自有线，来自 `9ae1c58 gpui: build the executor on gpui-pre 0.3.6, the GPUI gpui-kit pins`；参见 [Rust 分发](#rust-分发)）。`aeris_charts_render_gpui` 不再依赖 Zed 的 Git 修订：

- 此前，它使用来自 `https://github.com/zed-industries/zed` 的 `gpui` 0.2.2，修订为 `1057c2cf3d5b4aefd04755e1387c7826a4d7fba6`（其清单从 Zed 获取 GPUI，以跟进当前 1.0 之前的 API，并把 `gpui` 与 `gpui_platform` 固定在那一个经过评审的提交上）。现在，`gpui-backend` feature 启用 `gpui = { package = "gpui-pre", version = "=0.3.6" }`，一致性测试框架则使用 `=0.3.6` 的 `gpui-pre-platform`。清单把它记录为 gpui-kit 0.6.6（`gpui-component`）所固定的 `gpui`，因此使用 gpui-kit 0.6.6 的宿主应当已经解析到它；该固定不在本仓库中检查。
- 宿主必须解析到同一个包和同一版本：把其 Zed Git 依赖替换为 `package = "gpui-pre"`、`version = "=0.3.6"` 的形式（若使用平台 crate，则同时替换为 `gpui-pre-platform`）。仍停留在 Zed 修订上的宿主会同时持有两份 GPUI，每一个接收或返回 GPUI 类型的适配器与执行器调用（`input.mouse_down(&mut engine, &MouseDownEvent)`、`install_text_metrics(&mut engine, &Window)`、返回 `CursorStyle` 的 `cursor_style`）都无法针对宿主的那一份通过类型检查。
- 在 Windows 上保持启用 `windows-manifest` feature：GPUI 导入 `comctl32!TaskDialogIndirect`，它只有在该 feature 的清单所嵌入的 comctl32 v6 激活上下文下才能解析，因此没有它的可执行文件会在 `main` 之前加载失败（`STATUS_ENTRYPOINT_NOT_FOUND`）。
- `9ae1c58` 只改动清单、锁文件和文档：执行器与适配器源码未经修改即可针对 `gpui-pre` 0.3.6 编译，因此 Aeris 自身的 API 没有变化。宿主自己的 GPUI 代码不在本仓库中检查：迁移到 `gpui-pre` 0.3.6 是宿主需要自行完成并验证的变更。

**GPUI 快照 0.3.7**（自有线，即把 `gpui-pre` 升到 0.3.7、并把其余所有依赖升到最新发布版本的那个提交；可用 `git log -S'=0.3.7' -- crates/aeris_charts_render_gpui/Cargo.toml` 找到它）。它跟随 gpui-kit，后者已从 0.6.6 升到 0.7.0，并把 `gpui-pre =0.3.7` 与 `gpui-pre-platform`、`gpui-pre-web`、`gpui-pre-macros` 和 `gpui-pre-sum-tree` 固定在同一版本：

- 使用此执行器的宿主必须只使用一个 `gpui-pre` 版本，因此仍在 gpui-kit 0.6.6（`gpui-pre =0.3.6`）上的宿主，要在升级 Aeris 固定修订的同一次变更中迁移到 gpui-kit 0.7.0；两侧版本各异时，Cargo 会解析出两份互不兼容的 `gpui`，每一个接收或返回 GPUI 类型的适配器与执行器调用都无法通过类型检查，与上文较早的 Zed 修订的情形相同。
- Aeris 自身的 API 没有变化：执行器与适配器源码未经修改即可针对 0.3.7 编译，完整的 GPUI 测试套件也未经修改即通过。宿主自己的 GPUI 代码不在本仓库中检查。
- 一致性测试框架的 X11 捕获已迁移到 `x11rb` 0.14（仅为示例的 Linux 开发依赖）。GPUI 的 Linux 平台仍依赖 `x11rb` 0.13，因此示例构建同时包含两者；宿主所链接的内容不受影响。

**测量工具**（两条线，来自合并提交 `3eef45e merge: sync with AerisTerminal/aeris-charts main (range tools, input controller)`）。上游的 `5a2e6e8 feat(drawings): add price/date range measuring tools and Shift-click measure` 与自有线的 `36c9f09 feat(charts): B8 drawing catalog, multi-calendar overlays, bounded ticks, tick-built candles, and resampling` 各自独立地构建了这三个范围工具，合并保留了自有线的实现。固定修订采用哪种写法，取决于它属于哪一侧：

- 上游自 `5a2e6e8` 起的固定修订具有 `DrawingKind::DatePriceRange`、种类名称 `date_price_range`，以及线上 id 13（`PriceRange`）、14（`DateRange`）和 15（`DatePriceRange`）。自有线自 `36c9f09` 起的固定修订（例如 `36c9f09` 本身）已经具有 `DrawingKind::DateAndPriceRange`、名称 `date_and_price_range` 以及 id 130、131 和 132，这也是 main 所保留的。对自有线的固定修订而言，没有重命名，也没有 id 重映射；只适用下文的网格吸附。任一侧早于这些提交的固定修订都没有范围工具。
- 对上游的固定修订而言，`DrawingKind::DatePriceRange` 现在是 `DrawingKind::DateAndPriceRange`（`PriceRange` 与 `DateRange` 保持原名）。`DrawingKind::to_u8` 与 `from_u8` 的数值线上 id 已变动：`PriceRange` 为 130，`DateRange` 为 131，`DateAndPriceRange` 为 132，此前分别为 13、14 和 15。id 13 至 15 现为未分配，id 0 至 12 不变，因此存储了上游固定修订数值 id 的宿主需要重映射这些 id。
- 种类名称 `date_price_range` 仍会被读取（serde 别名和 `DrawingKind::from_name`），但绝不会被写出，因此由上游固定修订保存的文档仍可加载；保存的文档、模板和剪贴板载荷写出的是 `date_and_price_range`。
- 网格吸附。三个范围工具以及多头和空头仓位工具的创建、锚点拖动、主体拖动和键盘微调，都会吸附到整根柱以及品种 tick 或价格带价位梯。合并之前的自有线固定修订在这些工具上没有这种吸附，因此这五个工具都会获得这一变更（仓位工具的价格吸附更早，随合并 `2e7d19f` 引入）。上游自 `5a2e6e8` 起的固定修订已经让它们吸附到柱和价格 tick；它新增了价格带价位梯（`SeriesPriceFormat::tick_ladder`）。
- Shift 点击快速测量随 `5a2e6e8` 引入（合并之前的自有线固定修订从未具有它）。main 由输入控制器驱动它（在窗格上按下 Shift），因此无需为此调用 `measure_pointer_*`。

**十字光标遮罩、基线模式、实时柱缓动与时间线标记**（自有线，来自 `4f8a621 feat: live-bar easing, baseline reference line and mode, crosshair shade, timeline-mark lane`，即新增 `live_bar_easing_ms` 的提交，可用 `git log -S'live_bar_easing_ms' -- crates/aeris_charts_engine/src/lib.rs` 找到；参见[呈现扩展](presentation.md)）。每一项新增默认都关闭或为空，因此不采用任何一项的宿主渲染结果与之前相同；评审点是帧请求谓词，以及新增的枚举变体与结构体字段：

- 动画时钟。`ChartEngine::set_animation_time(ms)` 是引擎上的新方法（此前浏览器外壳自行记录一个时钟）：它推进每一个实时柱滑动，并仅在绘制了最新价脉冲或某个滑动推进时写入 `pub animation_time` 字段，因此仅打时间戳的 Tick 不改变任何帧键。`advance_live_bar_easing(now_ms) -> bool` 只推进滑动；`live_bar_easing_active()`、`animation_active()`（脉冲或滑动，即浏览器 `wants_animation` 的返回值）与 `animation_frame_requested()`（`input_animating() || live_bar_easing_active()`，Rust 宿主的帧请求谓词）都是新增的。按上文引擎输入控制器分组所述、仅在 `input_animating()` 成立期间请求下一帧的宿主，应改为 `animation_frame_requested()` 或 `GpuiChartInput::animating(&engine)`；否则滑动只显示第一帧便停滞，因为没有再请求帧。`GpuiChartInput::prepare_frame` 现在还会在适配器时钟上调用 `advance_live_bar_easing`，并在滑动移动时返回 `true`；适配器仍不运行脉冲时钟。
- 基线。`ChartEngine::series_baseline_price(id) -> Option<f64>` 与公共枚举 `BaselineMode`（`VisibleMidpoint`、`CloseBeforeVisibleRange`，带 `as_str` 与 `parse`）是新增的。`SeriesEntry` 新增 `pub` 字段 `baseline_mode`、`baseline_line_visible`、`baseline_line_color: Option<String>`、`baseline_line_width`、`baseline_line_style` 与 `live_bar_easing_ms`；列出全部字段的结构体字面量需要加上它们，默认值保持此前的渲染。直接写入 `live_bar_easing_ms` 的宿主得到 JSON 路径的语义：非有限或非正值为关闭，更大的值钳制到 `MAX_LIVE_BAR_EASING_MS`（1000）。
- 十字光标。`aeris_charts_core::options::CrosshairOptions` 新增 `shade_right: CrosshairShadeOptions { visible, color }`（线上键 `shadeRight`，`#[serde(default)]`，因此在它之前保存的 V2 文档仍可读取）；列出全部字段的结构体字面量需要加上它。
- 时间线标记。新增 `ChartEngine` 方法：`set_timeline_marks(snapshot) -> Result<(), ChartError>`、`timeline_marks()`、`set_timeline_marks_visible(bool) -> bool`、`timeline_marks_visible()`、`set_timeline_group_hidden(group, hidden) -> Result<bool, ChartError>`、`hidden_timeline_groups()`、`timeline_mark_hit_at(x, y)`、`timeline_mark_hit_at_with_profile(x, y, HitProfile)`、`timeline_mark_hit_for_id(id)`、`timeline_lane_pane()` 与 `timeline_mark_activation(seq)`；新增公共类型 `TimelineMark`、`TimelineMarkGlyph`、`TimelineGlyphShape`、`TimelineMarkGroup`、`TimelineMarksSnapshot` 与 `TimelineMarkHit`，以及上限 `MAX_TIMELINE_MARKS`（4,096）与 `MAX_TIMELINE_GROUPS`（64）。新增变体，每个都是穷尽 `match` 的编译期破坏性变更：`ChartInputEvent::TimelineMarkActivated(u32)`（与其他事件一起排空，并通过 `timeline_mark_activation(seq)` 读取命中）、`ChartHover::TimelineMark` 与 `InputTarget::TimelineMark`。`EngineMemoryUsage` 新增 `timeline_marks_capacity_bytes`。
- 持久化。V1、V2 与 V3 文档新增可选的 `hidden_mark_groups` 列表（为空时省略；无 schema 版本变更）。文档结构体忽略未知字段，因此本修订写出的带隐藏分组的文档在更早的固定修订上仍可加载，只是该列表被丢弃。
- Core。`PlotListView::with_row_override(source_row, [open, high, low, close])` 与 `overridden_values(row)` 是新增的，`value_at` 与 `is_whitespace_row` 遵循该覆盖；没有既有签名发生变化。浏览器包的新增均为增量的 `.d.ts` 成员（`crosshair.shadeRight`、基线/缓动系列选项、`series_api.baseline_price`、`chart_api.timeline_marks` 以及 `subscribe_timeline_mark_click` 一对），该提交中 npm 版本未变。

**其他源码级变更。** 每一项都注明携带该变更的提交。所涉及的公共枚举均不是 `#[non_exhaustive]`，因此每新增一个变体，对穷尽的 `match` 都是编译期破坏性变更；每新增一个字段，对列出全部字段的结构体字面量也是如此。

- `a565efc fix(kline): close K-line engine pitfalls across time, indicators, drawings, streaming, viewport, price axis, and intraday charts`（自有线）：
  - `PriceScaleCore::build_tick_marks` 与 `price_tick_span_calculator::composite_tick_span` 现在接收 `min_move: f64`（每个刻度所在的价格网格），此前接收的是 `base: i64`（格式化器的基数）。base 为 100 即 `min_move` 为 `0.01`，这也是引擎在百分比与指数化比例尺上的网格。
  - `SessionHighlightingOptions::start_hour_utc` 与 `end_hour_utc`（`Option<u8>`）现在是 `start_hour` 与 `end_hour`（`Option<f64>`）：带小数的交易所本地小时数（`9.5` 即 09:30；`start_hour > end_hour` 时跨越午夜）。`Some(9)` 变为 `Some(9.0)`，在交易所时间为 UTC 时含义相同。
  - `IndicatorKind::Ema`、`Dema`、`Tema` 与 `Rsi` 新增 `seed: IndicatorSeed`，`Macd` 新增 `seed` 与 `histogram_multiplier: f64`，`Bollinger` 新增 `estimator: DeviationEstimator`。`IndicatorSeed::Sma`、`1.0` 与 `DeviationEstimator::Population` 即此前的行为（同时也是 serde 默认值，因此已保存的文档仍可读取）；列出这些字段的模式需要加上 `..`。
  - `DrawingClipboardItem::points` 现在是 `Vec<DrawingAnchor>`，而不再是 `Vec<DrawingPoint>`（`DrawingAnchor { logical: Option<f64>, price, time: Option<f64> }`，可用 `DrawingAnchor::from(point)` 转换）。
  - 新增变体：`IndicatorKind::Kdj` 与 `IndicatorParameterType::Choice`。新增字段：`TimeScaleOptions::lock_visible_logical_range`、`PriceScaleCoreOptions::{ ensure_edge_tick_marks_visible, base_value, autoscale_center, stable_auto_scale}`、`PriceMark::edge`、`IndicatorInput::amount`、`SeriesPriceFormat::tick_ladder`、`SeriesEntry::histogram_updown_rule`、`IndicatorBindingInfo::amount_source`、`IndicatorParameterDescriptor::choices`，以及 `DrawingClipboardPayload` 和 `DrawingSyncPayload` 上的 `price_basis`。
- `36c9f09 feat(charts): B8 drawing catalog, multi-calendar overlays, bounded ticks, tick-built candles, and resampling`（自有线）：`ChartEngine::copy_drawings_json` 返回 `Result<String, ChartError>`，而不再返回 `Option<String>`（没有任何已知绘图可复制时返回 `ErrorCode::InvalidData`；超过 `MAX_DRAWING_CLIPBOARD_POINTS` 个锚点（最先检查）以及超过 `MAX_DRAWING_CLIPBOARD_BYTES` 字节时返回 `ErrorCode::ResourceLimit`）。上游的固定修订同样是 `Option<String>` 形式。新增变体：`DrawingKind`（B8 目录；自有线的后续提交又加入 `HorizontalSegment`、`VerticalRay`、`VerticalSegment`、`PriceChannel`、`PriceLine`、`SimpleTag` 和 `SimpleAnnotation`）、十个 `DrawingKindOptions` 变体、`DrawingDragPart::Handle(usize)`、`FootprintError::InvalidSessions`、`ResampleError::{InvalidSessions, TimeAxisRequired}`、`TradeStudyKind::Volume`，以及 `aeris_charts_core::model::plot_list::PlotValues::AsOf`。新增字段：`Drawing::tool_options`、`SeriesEntry::break_on_trading_day`，以及 `TradeStreamStats::{dependent_rows_computed, bar_rows_projected, bubble_trades_scanned, bubble_markers_sized}`。
- 自有线的后续新增：`ExchangeTimeError::BarTimeLabelWindows`（`78d7d59 feat(time): close-time display labels for open-stamped bars`）、`IndicatorKind::KLineChart`（`c2d837e engine: bind KLineChart indicators as chart studies`），以及 `DrawingTextEditLayout` 上的 `angle` 与 `multiline`；自有线自 `36c9f09` 起就有该类型（`d438dab feat(drawings): edit the text of every text-bearing tool and open the editor on placement`）。
- `960e011 feat(drawings): add complete position statistics and adaptive borders`（上游）：`DrawingKindOptions::Position` 新增 `account_size: f64` 与 `risk_percent: f64`，`Drawing` 新增 `position_account_size` 与 `position_risk_percent`。
- `5c62071 feat(engine): own native workspace identity`（上游）：`prepare_financial_frame_with_measure` 现在接收一个 `FinancialFrameRequest { width, height, dpr, force_layout, fit_content, frame, axis_primitives }`，其后是 `measure` 与 `countdown_measure`；此前它接收九个位置参数（`width`、`height`、`dpr`、`force_layout`、`fit_content`、`measure`、`countdown_measure`、`frame`、`axis_primitives`）；它仍返回 `FinancialFramePreparation`。
- 上游在分叉点 `ed2910d` 与 `1869773` 之间的提交还修改了这些既有的公共类型，因此不包含这些提交的固定修订，在其所匹配或构造的声明上会与此处存在差异：`TradingHitKind` 新增 `TakeProfitButton` 与 `StopLossButton`（`24e8e2d fix(trading): dedicated TP/SL buttons with pixel-exact, optically centered controls`），`TradingStyle` 与 `TradingStyleOptions` 新增 `execution_buy` 与 `execution_sell`，而 `ExecutionMarkerShape` 的默认值由 `Circle` 变为 `Arrow`（`6f56736 fix(trading): bar-anchored execution arrows with stacked multi-fill marks`）。范围种类与 `Position` 字段已在上文涵盖。早于 `ed2910d` 的固定修订还会与此前变更的类型存在差异，本节未列出这些类型。
