# 呈现扩展：十字光标遮罩、基线参考线、实时柱缓动与时间线标记

[文档导航](../README.md) · [架构总览](../Architecture.md) · [API 入口](README.md)

本页是提交 `4f8a621` 批次新增的四项引擎能力的使用契约。每一项默认关闭或为空，不采用任何一项的宿主渲染结果与之前相同。实现与不变量见[系列](../architecture/engine/series.md)、[帧契约](../architecture/rendering/frame.md)与[共享输入控制器](../architecture/engine/input.md)；Rust 宿主更换固定修订时的评审点见 [Rust 接入](rust.md#更换固定修订)。

- [十字光标右侧遮罩](#十字光标右侧遮罩)
- [基线参考线与基线模式](#基线参考线与基线模式)
- [实时柱缓动](#实时柱缓动)
- [时间线标记](#时间线标记)

## 十字光标右侧遮罩

```ts
chart.apply_options({
  crosshair: { shadeRight: { visible: true, color: "rgba(74, 74, 74, 0.12)" } },
});
```

**绘制内容。** 启用 `crosshair.shadeRight.visible` 后，引擎为悬停柱右侧的窗格区域加上遮罩：每个堆叠窗格一个填充矩形，从吸附柱的右边缘到窗格右边缘，覆盖窗格全高。该边缘与 `HighlightBarCrosshair` 图元使用同一条设备像素柱规则，因此柱高亮与遮罩恰好相接；遮罩跟随垂直线进入右侧的空白槽位，而当柱的右边缘取整后落在窗格边缘上（右偏移为 0 的最后一根柱配合奇数的设备柱宽）时不绘制任何内容。遮罩像垂直线一样绘制在每个窗格中，而不只是指针所在的窗格，并且独立于 `vertLine.visible`。它位于十字光标叠加层中，在系列、绘图、界面元素与交易对象之上、十字光标线之下，并遵循十字光标自身的门控：隐藏模式，以及交互对象（绘图、交易控件）被悬停或拖动时的抑制。移动指针只重建十字光标叠加层，与没有遮罩时相同。

**颜色。** `color` 是 CSS 颜色，其 alpha 承载不透明度；默认值 `rgba(74, 74, 74, 0.12)` 是 12% alpha 的十字光标线令牌，在两种主题下相同。无法解析的值（具名颜色、`hsl()`）回退到该默认值，而不是丢弃遮罩。遮罩是普通的半透明 `Rect`，因此继承每个执行器既有的 alpha 合成，没有遮罩专属的路径。WebGPU 执行器与 Canvas2D 完全一致的 source-over（`crates/aeris_charts_render_wgpu/src/blend.rs`）只对极淡的色调记录了 ±1/255 的残差：某通道预乘后的 8 位值取整为 0，即 `(c * a + 127) / 255 == 0`，其中 `a` 为 8 位 alpha；默认色调（`a = 31`，预乘后通道值为 9）不会触及它。GPUI 的半透明填充由仅作报告、不作门禁的半透明矩形一致性夹具覆盖。

**持久化与重置。** 该选项位于图表选项存储中，因此 V2 文档（通用窗格）与 `vertLine`/`horzLine` 一起携带它；V1 与 V3 文档不携带图表选项（见[持久化 V1](compatibility.md#持久化-v1)）。`chart.reset_style_to_defaults()` 恢复 `color` 并保留 `visible`，与水印相同的拆分。Rust 宿主读取 `CrosshairOptions::shade_right`（`CrosshairShadeOptions { visible, color }`）；没有单独的 setter，该选项与其他每个十字光标键一样经由 `apply_options` 流入。

## 基线参考线与基线模式

```ts
const price = chart.add_series("baseline", {
  baseline_mode: "close_before_visible_range",
  baseline_line_visible: true,                 // 默认 1 CSS px 的虚线 #4a4a4a
  baseline_line_color: "#4a4a4a", baseline_line_width: 1, baseline_line_style: 2, // 2 = dashed
});
price.baseline_price();                        // 象限所比较的价格，或 null
```

**哪个价格是基线。** 没有固定 `baseline_value` 的基线系列按 `baseline_mode` 从可见窗口解析其基线。`"visible_midpoint"`（默认值，即此前的行为）是可见收盘价最小值与最大值的中点。`"close_before_visible_range"` 是严格位于第一根可见柱之前的最后一个有限收盘价，因此窗口读作相对其起点的涨跌；窗口之前的空白数据行被跳过，没有任何有限行先于窗口时以第一个可见的有限收盘价代替，因此第一根柱读作不变，系列也不会消失。两种模式都跟随可见窗口，并随滚动或缩放变化；知道真实前一交易时段收盘价的宿主固定 `baseline_value`，它优先于每种模式。一次解析供给一切：象限填充与描边、参考线、实时价格线颜色、最新值坐标轴徽标、十字光标标记与 `baseline_price()`。该模式不改变 `histogram_updown_rule: "previous_close"` 的首根柱参考，后者仍只读取固定的 `baseline_value`（或比例尺的 `base_value`）。`"previous_close"` 不是 `baseline_mode` 的取值（它为未来以交易时段锚定的模式保留）；错误的值在应用本次调用的任何选项之前抛出 `invalid_options`，与 `histogram_updown_rule` 相同。

**参考线。** `baseline_line_visible`（默认 `false`）把解析出的基线绘制为一条全窗格宽的水平线，位于象限填充与象限描边之间，因此描边保持在其之上、它保持在填充之上。`baseline_line_color` 是按原文存储的 CSS 字符串；未设置或 `""` 跟随中性界面元素色调 `#4a4a4a`（十字光标线令牌，在两种主题下相同），无法解析的值回退到该色调而不是丢弃该线。`baseline_line_width` 以 CSS px 计（默认 `1`，须为正；按垂直像素比向下取整到整数设备像素，至少为一），`baseline_line_style` 是 `LINE_STYLE_TO_U8` 值（默认 `2`，虚线）。该线只在系列本身到达窗格时绘制：有一根柱在视野中，或有一段跨越窗口、连接两根屏幕外柱的线段；一旦系列完全滚出一侧，该线随其填充与描边一起消失。固定前收盘价时，它可以取代[分时图](intraday.md#分时图)示例中 `create_price_line({ price: prev_close, line_style: "dashed" })` 画线的那部分；该示例仍保留价格线，因为价格线还会在坐标轴上标出该价格，而参考线不带坐标轴标签。未固定时则无需任何宿主簿记。

**查询。** `series.baseline_price()`（`baselinePrice()`）以数字返回基线系列解析出的基线；对其他每种系列类型、已移除的系列、尚无可见范围的图表，以及没有任何一根柱位于可见窗口内（由另一系列保持窗口）的 `"visible_midpoint"` 系列返回 `null`。`"close_before_visible_range"` 此时仍有定义，报告窗口之前的最后一个收盘价，即系列完全向左滚出后它自己的最后一个收盘价。Rust 宿主调用 `ChartEngine::series_baseline_price(id)`；wasm 导出为 `AerisChart.series_baseline_price(id)`。

**重置、持久化与 worker。** 线条选项属于样式：`chart.reset_style_to_defaults()` 恢复它们。`baseline_mode` 是语义，与 `break_on_trading_day` 一样在重置后保留。与其他每个金融系列样式选项一样，它们都不持久化。worker（离屏）图表无法设置它们：worker 图表的 `apply_series_options` 在创建之后只接受对齐键，因此它们是主线程选项（见[分时图](intraday.md#分时图)）。

## 实时柱缓动

```ts
const price = chart.add_series("candlestick", { live_bar_easing_ms: 120 });
price.update({ time: last.time, open, high, low, close });   // 同一时间：已绘制的柱滑动
price.data().at(-1).close === close;                         // 立即为 true：查询读取真实值
```

**什么在滑动。** `live_bar_easing_ms` 是以毫秒计的时间常数 `tau`（默认 `0` 为关闭；高于 `1000` 的值钳制到 1000；负值或非有限值在应用本次调用的任何选项之前抛出 `invalid_options`）。当 `update()`、`update_typed()`、`merge()` 或合并批量就地替换已绘制的最后一根柱——同一规范行且同一时间——时，显示的最高、最低与收盘价在每个呈现帧上按 `x += (target - x) * (1 - exp(-dt / tau))` 向新值移动，其中 `dt` 是宿主时钟增量，每帧至多 100 ms，因此停止呈现的标签页恢复时以滑动而不是跳变继续。开盘价从不缓动。当每个通道都落在目标的 `max(1e-9, |target| * 1e-7)` 之内，或自最后一次变化起经过六个时间常数时，滑动精确落到目标上，动画循环停止；持续移动目标的数据源会一直滑动而不会冻结。全新的柱、同一时间集合上的 `set_data()`/`setData()`、`pop()`、移动的回放时钟、保留裁剪与 `reduced_motion` 交互选项都会立即跳到真实值。已绘制的柱滑动期间对另一根柱的写入（历史修正、超过回放时钟的回放柱）不打断滑动，因为已绘制的柱没有变化。被 `render_before_time` 隐藏的最后一根柱从不缓动。Tick 之后的第一帧仍显示先前的值（它记录时钟），因此滑动在第二个呈现帧上才可见。

**什么读取缓动后的值。** 只有为该柱绘制的内容：K 线、柱、直方图柱体、折线、面积或基线几何（`histogram_updown` 成交量柱体的着色取自主系列已绘制的方向，因此缓动后的收盘价穿过开盘价时它会翻转）、系列的最新值线与坐标轴徽标、脉冲、该柱上的十字光标标记（因此它落在已绘制的线上）以及系列命中测试（因此已绘制的影线可被命中）。Heikin Ashi 蜡烛用缓动后的原始值与前一个规范行重算其最后一行，因此同样滑动。其他一切始终读取真实值：`data()`、`data_by_index()`、`last_value_data()`、`bars_in_logical_range()`、图例/提示框/数据窗口背后的值快照、十字光标磁吸（磁吸模式下水平线吸附到真实收盘价，因此滑动期间与标记相差滑动量，这是一处有记录的单柱偏差）、交易几何、百分比/指数化基准值、`baseline_price()` 与自动缩放（它立即跟随真实范围）。

**宿主。** 浏览器图表不需要宿主代码：`wants_animation` 现在覆盖滑动，包的 rAF 循环（每次重绘后重新启动）运行到滑动落定。worker（离屏）图表创建后无法设置该选项，也不运行动画循环，因此从不缓动（见[分时图](intraday.md#分时图)）。Rust 宿主直接驱动 `ChartEngine`：`set_animation_time(ms) -> bool` 推进每一个滑动与脉冲时钟并报告该步是否改变了所绘内容（浏览器循环与 GPUI 适配器的 `GpuiChartInput::prepare_frame` 都调用它），`advance_live_bar_easing(now_ms) -> bool` 只推进滑动，`animation_active()` 是浏览器循环的谓词，`animation_frame_requested()` 是 Rust 宿主的帧请求谓词（输入动画或尚未落定的滑动；绘制中的脉冲由时钟步自身保持帧，因为它每帧都报告变更）。`live_bar_easing_active()` 回答是否有任何滑动尚未落定。宿主契约见[浏览器边界](../architecture/hosts/browser.md#输入订阅与动效)与 [GPUI 后端](../architecture/rendering/backends.md#gpui)。

**重置、持久化与一致性。** 该选项是样式，与 `last_price_animation` 一样：`chart.reset_style_to_defaults()` 关闭它，且它不持久化。落定后的帧与干净重建的帧、以及仅由真实值构建的帧逐位相同；滑动中的帧取决于宿主时钟，因此像素一致性夹具保持该选项关闭。

## 时间线标记

```ts
chart.timeline_marks().set({
  groups: [{ id: "earnings", label: "Earnings" }],
  marks: [
    { id: "q3", time: 1_700_000_000, group: "earnings", glyph: { shape: "circle", color: "#2962ff", letter: "E" }, title: "Q3 report" },
    { id: "call", time: 1_700_000_000, group: "earnings", title: "Call" },      // 同一根柱：一个计数为 2 的 token
    { id: "up", time: 1_700_300_000, group: "news", glyph: { shape: "diamond", color: "#f7525f" } }, // `groups` 中缺失的分组以其 id 为标签
  ],
});
chart.subscribe_timeline_mark_click((hit) => open_popup(hit.mark_ids, hit.label, hit.title));
chart.timeline_marks().set_group_hidden("news", true);  // 随 export_state() 持久化
```

**标记带是什么。** 沿主系列窗格底部的一行字形 token：每个柱槽位（或每簇相近槽位）一个 token，与系列标记和交易宿主叠加层分离。一个标记有唯一的 `id`（1..=128 字节）、unix 秒 `time`、分组 `group` 的 id、字形（`shape`：`circle`（默认）、`square`、`diamond` 或 `pin`；CSS `color`；至多两个字符的 `letter`）以及 `title`（至多 128 字节）。分组携带在提示框中显示的 `label`（至多 64 字节）；标记可以指向 `groups` 中缺失的分组，该分组此时以其 id 为标签。`set()` 原子地替换标记与分组：超过 4096 个标记或 64 个不同分组时抛出 `resource_limit`，重复 id、过长字符串或无法解析的颜色时抛出 `invalid_data`，标记带保持不变。`set_visible(false)` 隐藏整条标记带（仅运行时，默认显示）。

**标记落在哪里。** 引擎解析柱槽位：时间落入其跨度 `open .. open + min(step, next_open - open)` 的那根柱，其中 `step` 依次取已安装交易时段柱网格的间隔（收盘时间标签）、`set_future_time_projection` 的节奏、当前的柱间隔。位于缺口——隔夜或周末——中的时间落到下一根柱上，绝不落到缺口之前的柱上；超过最后一根柱的时间按整数个 step 投影到右侧空白区，且只在比例尺显示该空白区时绘制；第一根柱之前或回放时钟之后的时间不绘制任何内容。tick、成交量与 range 柱按柱的 open..close 跨度映射，不做投影。中心落在某簇最早槽位 20 CSS px 之内的 token 折叠为一个 token：单一分组的簇保留该分组的字形并以计数取代字母，混合分组的簇绘制一个带边框的中性方块与计数（从一百起为 `99+`）。聚类在 CSS px 中计算，因此在每个设备像素比下相同；token 几何源于常量而非度量文本，因此标记带在每个后端上像素一致。标记带在窗格高度低于 96 CSS px 时隐藏，高于 108 CSS px 时再显示（迟滞）；显示且有任何标记时，它在所在窗格每个自动缩放的比例尺上为数据下方预留 27 CSS px——与视图及隐藏分组无关，因此平移或切换分组绝不会移动比例尺；手动比例尺可以像系列标记一样与标记带重叠，钉在底部的成交量叠加层随之抬升。

**交互。** 悬停 token 得到 pointer 光标并绘制一个 1 px 的环；经过与交易控件相同的 450 ms 停留后，引擎以主题的表面、边框与文字令牌绘制标题提示框（单个标记为 `label · title`，单一分组的簇为 `label · N`，混合簇为 `N marks`）。一次点击（在同一 token 上按下并释放且未拖动）触发一次 `subscribe_timeline_mark_click`，携带 `timeline_mark_hit`——锚点 `logical` 槽位与 `projected` 标志、最早的 `time` 与 `title`、`count`、不同的 `groups`、每个 `mark_ids` 条目以及 `label`——而绝不选择、平移或到达其下方的绘图；token 上的双击绝不会打开绘图编辑器。`hit_at(x, y)` 对任意图表内容坐标回答同样的精确命中：token 在自身方框内响应（16 CSS px，簇为 18），标记带其余部分属于窗格。只有图表上的触摸按下才在引擎输入控制器内按触摸命中容差加宽该方框；`hit_at` 本身始终使用精确方框。丰富的点击弹层仍是宿主 UI。无障碍为已显示分组的每个标记暴露一个 `mark:` 焦点目标，位于引擎显示标记带的窗格上（标记带启用、非空且足够高时为主系列的窗格），标记带隐藏时没有目标，其标签即提示框文本；标记在视野中时，Enter 通过同一个点击订阅激活 token。

**隐藏分组与持久化。** `set_group_hidden(group, hidden)` 隐藏或显示一个分组的标记（至多 64 个隐藏分组，128 字节 id；抛出 `resource_limit`/`invalid_data`）。该集合独立于快照，因此宿主可以先导入文档、后设置标记，新的 `set()` 绝不裁剪它。隐藏分组是标记带唯一持久化的部分：V1、V2 与 V3 文档上可选的 `hidden_mark_groups: string[]` 字段（为空时省略，无 schema 版本变更；超过 64 项或 id 过长的文档在结构层面原子地失败），见[持久化 V1](compatibility.md#持久化-v1)。标记本身从不持久化。

**宿主。** Rust 宿主调用 `ChartEngine::set_timeline_marks`、`timeline_marks`、`set_timeline_marks_visible`、`timeline_marks_visible`、`set_timeline_group_hidden -> Result<bool, ChartError>`、`hidden_timeline_groups`、`timeline_mark_hit_at`、`timeline_mark_hit_at_with_profile(x, y, HitProfile)`（触摸容差）、`timeline_mark_hit_for_id`、`timeline_lane_pane -> Option<usize>`（当前显示标记带的窗格；标记的键盘目标跟随它），并通过 `timeline_mark_activation(seq)`（32 项的环）读回点击的 `ChartInputEvent::TimelineMarkActivated(seq)`。wasm 导出为 `set_timeline_marks_json`、`timeline_marks_json`、`set_timeline_marks_visible`、`set_timeline_group_hidden`、`hidden_timeline_groups_json`、`timeline_mark_hit_json`、`timeline_mark_hit_for_id_json` 与 `timeline_lane_pane`；控制器事件 `timeline_mark_activated` 携带已解析的 `hit`。worker（离屏）图表在本修订中不代理该标记带句柄。
