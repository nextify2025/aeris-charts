# Aeris Charts 架构

[文档导航](README.md) · [公共 API](api/README.md) · [验证门禁](development/validation.md)

## 目的与阅读方式

Aeris Charts 为 [Aeris Terminal](https://aeristerminal.com) 和浏览器宿主提供确定性的图表状态、交互、绘图、指标与多后端渲染。同一份规范状态必须在 GPUI、WebGPU、Canvas2D 和原生渲染之间生成等价的帧；性能、视觉一致性、确定性和有界资源都是产品要求。

本页是架构入口，详细契约按职责放在 `architecture/` 的不同目录中。先读本页，再沿下表阅读任务涉及的主题；不要把 API 使用说明、未来计划或机器测量结果当作当前架构。源代码、测试和实测的 release 行为是事实依据，变更须在同一次提交中更新对应主题；只有职责、依赖或导航变化时才需同时修改本页。

## 数据流

```text
宿主 API / 行情数据 / 平台输入
    ├─ 浏览器：packages/charts → aeris_charts_wasm
    └─ Rust：直接调用 ChartEngine（GPUI 输入经 GpuiChartInput）
        ↓
aeris_charts_engine：校验、规范状态、交互、指标绑定与失效
    ├─ aeris_charts_core：数据、比例尺、选项与共享数学
    └─ aeris_charts_indicators：纯指标计算与增量运行时
        ↓
ChartFrame + aeris_charts_render 的有序 DrawList
        ↓
GPUI | WebGPU | Canvas2D | 原生 tiny-skia
        ↓
像素、帧指标与宿主事件
```

数据与输入在宿主边界转换，不在宿主或执行器内重建图表语义。浏览器句柄与 React 适配器调用同一个引擎；任意宿主扩展通过显式的回调边界贡献帧内容，不构成第二个场景模型。

## Crate 边界

下表按生产依赖与实际所有权划分。尤其注意：通用坐标轴、数据集和系列注册表属于 **engine**，不是 core。

| Crate / 包 | 拥有的职责 | 详细契约 |
| --- | --- | --- |
| [`aeris_charts_core`](../crates/aeris_charts_core/Cargo.toml) | 规范金融数据、比例尺数学、时间、选项、格式化 | [数据](architecture/data/storage.md)、[时间](architecture/data/time.md)、[比例尺](architecture/engine/panes-and-scales.md#核心比例尺数学) |
| [`aeris_charts_indicators`](../crates/aeris_charts_indicators/Cargo.toml) | 纯公式、预热、滚动状态与检查点；不认识图表或平台 | [指标计算与绑定](architecture/data/indicators.md) |
| [`aeris_charts_engine`](../crates/aeris_charts_engine/Cargo.toml) | 图表状态、金融与通用系列、窗格、交互、绘图、交易、持久化和帧构建 | [引擎领域](#引擎领域) |
| [`aeris_charts_render`](../crates/aeris_charts_render/Cargo.toml) | 后端中立的图元、共享几何、Canvas2D 执行契约与有序绘制列表 | [共享图元](architecture/rendering/primitives.md) |
| [`aeris_charts_render_gpui`](../crates/aeris_charts_render_gpui/Cargo.toml) | GPUI 场景转换、缓存、度量，以及唯一的 GPUI 输入适配器 | [后端](architecture/rendering/backends.md#gpui)、[输入](architecture/engine/input.md) |
| [`aeris_charts_render_wgpu`](../crates/aeris_charts_render_wgpu/Cargo.toml) | WebGPU 编码、图集、混合、裁剪与 GPU 资源 | [WebGPU](architecture/rendering/backends.md#webgpu) |
| [`aeris_charts_wasm`](../crates/aeris_charts_wasm/Cargo.toml) | WASM 输入转换、浏览器呈现、Canvas2D/WebGPU 策略与遥测 | [浏览器边界](architecture/hosts/browser.md) |
| [`aeris_charts_native`](../crates/aeris_charts_native/Cargo.toml) | tiny-skia 渲染、原生图像导出、golden 与 release 性能门禁 | [原生执行](architecture/rendering/backends.md#原生渲染与图像导出) |
| [`packages/charts`](../packages/charts/package.json) | TypeScript 句柄、DOM 生命周期、平台效果、可选 React 适配器 | [宿主边界](architecture/hosts/browser.md)、[公共 API](api/README.md) |

所有 Rust crate 都是 `publish = false`，宿主通过固定 Git 修订或路径依赖使用。工作区采用 Rust 2024 版次与 resolver 3，每个 crate 都继承工作区的 `rust-version = "1.99"`，因此宿主需要 rustc 1.99 或更高版本（见 [Rust 分发](api/rust.md#rust-分发)）。浏览器包 `@aeristerminal/aeris-charts` 是唯一发布的产物。GPUI 执行器当前固定 `gpui-pre =0.3.7`，宿主必须使用同一包与版本；历史升级说明见 [Rust 接入](api/rust.md)。CI 另以 gpui-fast 构建并测试该执行器，作为证据而非发布门禁（见 [gpui-fast 证据线](development/validation.md#gpui-fast-证据线)）。

## 依赖方向

以下箭头表示“依赖”，不是帧的流动方向；省略第三方库及测试依赖：

```text
render      → core
engine      → core + indicators + render
render_gpui → core + engine + render
render_wgpu → render
native      → core + engine + render
wasm        → core + engine + render + render_wgpu
```

`core`、`indicators`、`engine` 和 `render` 不依赖浏览器、GPUI 或产品应用。下层不得导入宿主 API 绕过边界；后端不得分叉图表语义。新增 crate、trait 或功能开关必须保护当前真实存在的边界，而不是为假想的实现预留接口。

## 引擎领域

| 修改内容 | 阅读主题 |
| --- | --- |
| 系列数据、批量、as-of 对齐、摘要与保留 | [规范数据](architecture/data/storage.md) |
| UTC 时间、交易日、时段槽位、收盘标签、视口重新定基 | [时间](architecture/data/time.md) |
| 公式、预热、绑定、成交量分布、周期分布、TPO 与分布绘图、外部研究与指标 V3 | [指标](architecture/data/indicators.md) |
| 结构与时段研究、时段日历、只读研究注释 | [结构与时段研究](features/studies.md) |
| 金融窗格、命名比例尺、坐标、自动缩放与选项 | [窗格与比例尺](architecture/engine/panes-and-scales.md) |
| 非金融域、坐标轴、类型化数据集、通用系列与交互 | [通用图表](architecture/engine/general-charts.md) |
| 指针、触摸、滚轮、键盘、悬停、取消与动效 | [共享输入控制器](architecture/engine/input.md) |
| 金融系列几何、基线解析、实时柱缓动、时间线标记带、值快照、官方图元与图例 | [系列与图元](architecture/engine/series.md) |
| 成交流、深度、回放、非时间柱与重采样 | [行情投影](architecture/engine/market-data.md) |
| 持仓、订单、成交、警报、意图与回滚 | [交易与警报](architecture/engine/trading.md) |
| 绘图状态、时间锚点、历史、复权与测量 | [绘图](architecture/engine/drawings.md) |
| 工具族、部件、wire id、schema、旧版工具名与文档迁移、新增工具流程 | [绘图族](architecture/engine/drawing-families.md) |
| 文本布局、命中、编辑会话、输入法与插入符 | [绘图文本](architecture/engine/drawing-text.md) |
| 语义文档、工作区拓扑、恢复与图表联动 | [持久化与工作区](architecture/engine/persistence.md) |

## 状态与帧的所有权

每个图表只有一个引擎所有者。规范数据、语义绘图与宿主权威的业务输入，与可丢弃的查询索引、保留图层及平台资源分开。引擎拥有失效、布局、自动缩放与帧准备；后端只执行同一份有序帧，并在输出不变的前提下复用资源。

- [帧契约](architecture/rendering/frame.md)：代次、坐标修订、图层顺序、选择与控件。
- [共享图元](architecture/rendering/primitives.md)：`f64` 几何、像素对齐、描边、图像与旋转文本。
- [后端](architecture/rendering/backends.md)：GPUI/WebGPU/原生执行、图集、设备丢失与回退。

## 插件与宿主扩展

宿主获得行为，不负责拼装引擎机制。平台输入转换、捕获、光标应用、定时器、菜单、剪贴板、产品持久化和券商操作仍在宿主；交互仲裁与命中、绘图编辑和语义查询留在引擎。

详见[浏览器与 React 边界](architecture/hosts/browser.md)和[扩展边界](architecture/hosts/extensions.md)。Aeris Terminal 是独立仓库，修改 Charts 不代表已经验证了 Terminal 的集成。

## 性能契约与验证

- [性能契约与证据](development/performance.md)：有界工作量、稳态资源、基准子系统与产物体积。
- [验证门禁](development/validation.md)：固定工具链、Rust/包/浏览器/GPUI 检查，以及机器证据能证明和不能证明的内容。
- [基准测试指南](../benchmarks/README.md)：场景、命令、基线、预算与结果格式。
- [贡献指南](development/contributing.md)：贡献与文档维护约定。

保留算法、正确性、恢复和验证限制，不能以文档精简为由删除。当前架构记在对应主题，未来目标留在 [`plan/`](../plan/plan.md)，生成报告不提交到文档目录。
