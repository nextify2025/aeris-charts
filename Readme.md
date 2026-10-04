# Aeris Charts

Aeris Charts 是面向 [Aeris Terminal](https://aeristerminal.com) 和浏览器宿主的 Rust 图表引擎。同一个确定性图表模型驱动 WebGPU、Canvas2D、GPUI 和原生渲染。

该项目包含专业的图表交互、绘图、技术指标、多窗格与多比例尺、自定义系列、图元、共享内存行情数据输入，以及后端一致性工具。

## Rust crate

Rust 宿主通过 Git 或路径依赖，从本仓库依赖 `aeris_charts_*` crate；这些 crate 不会发布到 crates.io。`aeris_charts_engine` 拥有图表状态、交互、绘图、指标和帧构建，宿主将其与渲染器（例如 `aeris_charts_render_wgpu` 或 `aeris_charts_native`）搭配使用（参见[仓库结构](#仓库结构)）。

## 浏览器包

浏览器 SDK 通过 GitHub Packages 以 `@aeristerminal/aeris-charts` 发布。安装前请先配置 Aeris Terminal 作用域：

```text
@aeristerminal:registry=https://npm.pkg.github.com
```

```sh
npm install @aeristerminal/aeris-charts
```

在配置好发布凭据且标签与 `packages/charts/package.json` 完全一致后，版本标签会自动发布（例如，包版本 `0.9.0` 由标签 `v0.9.0` 发布）。

使用异步的驼峰式 API 创建图表：

```ts
import { createChart } from "@aeristerminal/aeris-charts";

const container = document.querySelector<HTMLElement>("#chart");
if (!container) throw new Error("missing chart container");

const chart = await createChart(container, { autoSize: true });
const candles = chart.addSeries("candlestick");

candles.setData([
  { time: 1735689600, open: 100, high: 108, low: 98, close: 105 },
  { time: 1735776000, open: 105, high: 112, low: 103, close: 110 },
]);

chart.timeScale().fitContent();
```

原有的蛇形命名在同一个图表和系列句柄上仍然可用。选项和数据字段保留其文档中记载的名称；驼峰式别名适用于常用的方法调用。

金融系列和通用系列可以共享同一个图表生命周期，同时分别占用坐标域兼容的窗格：

```ts
const summary = chart.addPane({
  preserve_empty: true,
  horizontal_domain: { type: "category", scale: "band" },
});
const pane = summary.paneIndex();
chart.addAxis({ id: "month", pane, dimension: "x", scale: "band" });
chart.addAxis({ id: "revenue", pane, dimension: "y", scale: "linear" });
const revenue = chart.addSeries("column", {
  pane, x_axis_id: "month", y_axis_id: "revenue", title: "Revenue",
});
revenue.setData([{ id: "jan", x: "Jan", y: 42 }, { id: "feb", x: "Feb", y: 57 }]);

// Both panes render through the same engine and chart.render() lifecycle.
chart.render();
```

同一引擎还提供可选的 React 编写层。使用它的应用需安装 React，然后从包子路径导入适配器；与框架无关的应用不会加载 React，也不依赖 React：

```sh
npm install @aeristerminal/aeris-charts react
```

```tsx
import { FinancialSeries, GeneralPane, AerisChart } from "@aeristerminal/aeris-charts/react";

const axes = [
  { id: "month", dimension: "x", scale: "band" },
  { id: "revenue", dimension: "y", scale: "linear" },
] as const;

export function Dashboard({ candles, revenue }) {
  return (
    <AerisChart options={{ autoSize: true }} style={{ width: "100%", height: 560 }}>
      <FinancialSeries kind="candlestick" data={candles} />
      <GeneralPane
        options={{ horizontal_domain: { type: "category", scale: "band" } }}
        axes={axes}
        series={[{
          key: "revenue",
          kind: "column",
          options: { x_axis_id: "month", y_axis_id: "revenue", title: "Revenue" },
          data: revenue,
        }]}
      />
    </AerisChart>
  );
}
```

适配器只创建一次普通的命令式图表，将数据与配置协调到保留的引擎句柄上，并在卸载时调用相同的 `chart.remove()` 生命周期。其模块可在 SSR 期间安全导入，因为图表创建和 DOM 访问仅在组件挂载之后才开始。完整的框架无关与 React 组合示例位于 `examples/all_in_one/`。

优化后的 WASM 二进制文件与 ESM 入口一同分发，并自动在该位置解析。需要显式资源 URL 的打包器可以导入 `@aeristerminal/aeris-charts/wasm`（或该导出对应的常规 URL 加载器形式），并在创建图表之前将得到的 URL 传给 `initWasm()`；`pkg/`、`crates/`、演示或仓库路径均不属于使用方契约。

数值时间是有限的整数 UTC 秒，取值范围为闭区间 `-62167219200..253402300799`（年份 0000..9999）。Aeris 绝不自动转换数值时间戳；拒绝时会在适用的情况下附带可能是毫秒、微秒或纳秒的提示。直接的 set/update 批量写入在出现任何无效时间戳时会原子地整体拒绝，无效的单条更新则保持现有数据不变。原因请检查 `series.last_ingestion_diagnostics()`。

## 高级图表功能

高级金融系列是一等的 Rust 引擎系列。它们的数据、自动缩放投影、几何、生命周期和渲染由每个后端共享；浏览器包仅在 WASM 边界处转换公共数据和选项：

```ts
import { create_volume_profile } from "@aeristerminal/aeris-charts";

const heatmap = chart.add_series("heatmap", {
  cell_border_width: 1,
  cell_border_color: "rgba(255,255,255,.08)",
  cell_shader: (amount) => `rgba(80,0,255,${Math.min(1, amount / 100)})`,
});
heatmap.set_data(heatmap_data);

const profile = create_volume_profile(candles, {
  time: 1735689600,
  profile: [{ price: 100, vol: 12 }, { price: 101, vol: 28 }],
  width: 10, // time-scale bar slots
});
// profile.set_data(next_time_anchored_profile); profile.detach();
```

引擎拥有的功能集包括可刷选面积图、分组柱、热力图、HLC 面积图、美化直方图、阴影背景、堆叠面积图、堆叠柱以及箱线图系列。图元辅助函数包括无障碍、锚定文本、官方 ±10% 价格带、差值提示框与普通提示框、高亮柱十字光标、图像水印、叠加价格比例尺、局部价格线、矩形/趋势线/垂直线绘图、交易时段高亮、成交量分布以及用户自定义价格线。

线周围热力图与阴影背景示例叠放在普通折线系列下方。

Aeris 已经拥有的功能——绘图（包括 Long Position 和 Short Position 工具、价格区间、日期区间以及日期与价格区间测量工具，还有 Shift 点击快速测量）、价格带、价格线、叠加比例尺、局部最新价格线、交易时段着色、高亮柱槽位以及按时间锚定的成交量分布——都是这些引擎 API 之上的薄辅助层。无障碍默认启用；`chart.accessibility()` 返回其单例控制器，`enable_accessibility(chart, options)` 为保持兼容而配置同一个实例。键盘/ARIA 节点和播报仍属于浏览器 DOM 外壳，而有界的数据查询、焦点几何、绘图编辑和渲染图元则使用共享引擎。除非启用 `announce_data_updates`，流式行情更新保持静默。每个带有 `detach()` 的返回功能句柄都会释放其引擎状态和宿主状态。

浏览器输入对鼠标和触控笔使用 Pointer Events，并对动态页面滚动仲裁使用可取消的 Touch Events。引擎拥有有界的手势状态、5 px 拖动阈值、固定起始质心的累计捏合行为、主触点延续、取消，以及感知设备的命中容差。滚轮策略可通过 `wheel_behavior: "auto" | "pan" | "zoom"` 配置；参考固定的公共参考实现夹具中测得的行为，auto 在窗格或任一坐标轴上独立地根据垂直增量缩放时间、根据水平增量平移时间，没有 Ctrl/Shift 特例。显式的 `pan` 和 `zoom` 值保留 Aeris 的扩展路由。

## 交易与订单管理

交易对象是独立的自有引擎领域。应用提供权威的持仓、挂单、括号/OCO 关系、成交以及品种元数据；Aeris 拥有它们的确定性可视化、原生坐标轴标签、命中测试、风险/回报区域以及本地交互预览。拖动绝不会改写已确认的券商状态。即时模式在释放时发出一个类型化的、与券商无关的意图；手动模式则将预览保留在内联的 Confirm 和 Discard 控件之后。宿主用已接受的状态更新来协调已确认的预览，或显式拒绝它。风险/回报填充仅属于活动预览，绝不属于已确认的订单。

```ts
const trading = chart.trading();
trading.set_confirmation_mode("manual"); // Optional; the default is "instant".
trading.apply_snapshot({
  instrument: { tick_size: 0.25, price_precision: 2, point_value: 50, currency: "USD" },
  positions: [{ id: "position-1", side: "long", average_price: 5230, quantity: 2 }],
  orders: [{
    id: "target-1", side: "sell", kind: "limit", role: "take_profit", status: "working",
    price: 5240, quantity: 2, position_id: "position-1", oco_group_id: "bracket-1", revision: 4,
  }],
});

trading.subscribe_intents(async (intent) => {
  const accepted = await route_to_broker(intent);
  trading.resolve_intent(intent.sequence, accepted);
  // On acceptance, push the resulting authoritative order/position update through this API.
});

// Convert a completed Long/Short Position drawing into one atomic bracket request. Quantity is
// deliberately host-owned; the intent carries the drawing's tick-snapped entry, TP, and SL.
const plan = chart.selected_drawing();
if (plan && (plan.kind() === "long_position" || plan.kind() === "short_position")) {
  trading.place_bracket_order(plan.id, quantity_from_host);
}
```

实时交易对象、预览和意图队列是图表本地的运行时状态，并且有意不包含在 `chart.export_state()` 中。

请为容器指定明确的尺寸；图表画布会填满该容器。

在浏览器宿主中导入一次可移植的设计系统：

```ts
import "@aeristerminal/aeris-charts/design.css";
```

浅色是 CSS 的默认主题。在根元素上设置 `data-theme="dark"`（或类 `dark`）即可启用深色模式，并对图表应用 `theme_options("dark")`。宿主外壳和图表标签默认使用系统 UI 字体栈。图表字体仍是显式的布局选项，因此在宿主设置它之前，宿主的网页字体不会使金融标签发生偏移。图表默认值直接使用相同的语义角色：坐标轴和数值文本使用 foreground，十字光标标签表面使用 muted。十字光标线在两种主题中均使用与主题无关的 `crosshair_line` 令牌（`#4a4a4a`）。

## 仓库结构

- `crates/aeris_charts_core`——已验证的数据、比例尺、选项、格式化和共享数学运算。
- `crates/aeris_charts_indicators`——与平台无关的指标计算。
- `crates/aeris_charts_engine`——图表状态、交互、绘图、窗格和帧构建。
- `crates/aeris_charts_render`——后端中立的图元和有序绘制列表。
- `crates/aeris_charts_render_wgpu`——WebGPU 执行器。
- `crates/aeris_charts_render_gpui`——GPUI 执行器。
- `crates/aeris_charts_wasm`——浏览器与 WebAssembly 边界。
- `crates/aeris_charts_native`——确定性原生渲染与性能验证。
- `packages/charts`——TypeScript 浏览器包。
- `examples/web_demo`——浏览器集成与一致性测试宿主；它不是已发布的包。
- `docs`——架构、公共 API、领域模型和贡献文档。
- `plan`——现行的产品与扩展计划。

有关所有权、数据流和后端边界，参见 [Architecture.md](docs/Architecture.md)。有关受支持/实验性接口、持久化、错误和版本策略，参见 [Public_api.md](docs/Public_api.md)。

## 开发

前置条件：稳定版 Rust、`wasm32-unknown-unknown` 目标、`wasm-pack`、Bun，以及 Node.js 18 或更新版本（`node` 脚本和 Playwright 在 Node 上运行）。

```sh
cargo test --workspace

cd packages/charts
bun install --frozen-lockfile
bun run build
bun run lint
bun run typecheck
bun run test:pack
```

完整的验证门禁记录在 [AGENTS.md](AGENTS.md) 中，并由 CI 强制执行。贡献要求记录在 [CONTRIBUTING.md](docs/CONTRIBUTING.md) 中。

## 性能证据

可复现的 release 包基准测试位于 [`benchmarks/`](benchmarks/README.md)。该测试框架记录确定性工作负载、原始样本、统计汇总、构建与机器来源信息、能力限制、包体积、浏览器 CPU/GPU 耗时、内存、生命周期、伸缩性以及长时间运行行为。共享 CI 的结果仅作诊断之用；只有受控基准测试环境中的干净运行才可以产出公开声明或发布基线。

## 许可证

Aeris Charts 是开源软件，依据 [GNU Affero General Public License v3.0](LICENSE) 授权，其 SPDX 表达式为 `AGPL-3.0-only`。AGPL 允许商业使用、修改和再分发，但须遵守其 copyleft 与对应源代码要求，包括其中关于网络交互的条款。

无法遵守 AGPL 的组织，可以获取单独的 Aeris Terminal Commercial License，用于专有集成、再分发、OEM/嵌入式使用、白标使用、支持和定制工程。商业选项是一份单独的协议；它不会为公开的 AGPL 授权增加任何限制。参见 [COMMERCIAL_LICENSE.md](COMMERCIAL_LICENSE.md)。

## 独立开发与第三方参考

Aeris Charts 是独立设计和实现的。公开文档、公开示例以及对成熟图表产品的行为观察，仅用于了解常见的用户期望，并构建仅限开发用途的兼容性对比。这些参考资料不与 Aeris 共享引擎、渲染或状态管理实现。

KLineChart 指标移植是例外：其公式转译自 [KLineChart](https://github.com/klinecharts/KLineChart) v10.0.3（Apache-2.0），并在 [NOTICE](NOTICE) 和模块文档中注明出处。

开发测试通过 Lightweight Charts 的公共 API，将其作为固定版本的 Apache-2.0 依赖使用。该依赖不包含在已发布的 `@aeristerminal/aeris-charts` 包中。TradingView 和 Lightweight Charts 是其各自所有者的商标；Aeris Charts 与 TradingView 没有隶属关系，也未获得 TradingView 的认可。参见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
