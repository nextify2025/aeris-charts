# 文档导航

[项目介绍与快速开始](../Readme.md) · [架构总览](Architecture.md) · [公共 API](api/README.md)

## 从哪里开始

| 目标 | 入口 |
| --- | --- |
| 在浏览器或 React 中接入图表 | [公共 API](api/README.md) |
| 在 Rust / GPUI 中接入或升级固定修订 | [Rust 接入](api/rust.md) |
| 修改引擎、交互、渲染或跨 crate 行为 | 先读[架构总览](Architecture.md)，再读涉及的主题 |
| 搭建开发环境、提交变更与验证 | [贡献指南](development/contributing.md)、[验证门禁](development/validation.md) |
| 测量性能或核对预算 | [性能契约](development/performance.md)、[基准测试指南](../benchmarks/README.md) |
| 查看未完成工作与验收目标 | [`plan/plan.md`](../plan/plan.md)、[扩展计划](../plan/Expansion.md)、[感知计划](../plan/Perception.md) |

## 架构目录

架构按职责拆分，不把所有实现细节堆在 crate 简介中：

```text
docs/
├── Architecture.md          架构总览、数据流、crate 与依赖地图
├── architecture/
│   ├── data/                规范数据、时间与指标计算/绑定
│   ├── engine/              窗格、系列、交互、绘图、交易与持久化
│   ├── rendering/           帧、图元、执行器与资源生命周期
│   └── hosts/               浏览器、React 与扩展边界
├── api/                     使用契约、示例、兼容性与 Rust 迁移
├── features/                足迹图、深度等领域设计
└── development/             贡献、验证门禁与性能证据
```

具体主题与阅读顺序见[架构总览](Architecture.md#引擎领域)。目录按职责组织，不意味着新增了 crate 或运行时层。

## API 主题

- [指标、KLineChart 模板与成交量分布](api/indicators.md)
- [坐标、窗格、时区与收盘时间标签](api/coordinates-and-time.md)
- [分时图](api/intraday.md)
- [成交构柱与重采样](api/aggregation.md)
- [绘图锚点、磁吸、复权与工具目录](api/drawings.md)
- [通用图表：已实现契约与后续提案](api/general-charts.md)
- [呈现扩展：十字光标遮罩、基线参考线、实时柱缓动与时间线标记](api/presentation.md)
- [兼容性、错误、生命周期与持久化](api/compatibility.md)
- [Rust 接入与固定修订升级](api/rust.md)

受支持接口和实验性接口以 [API 入口](api/README.md)及其兼容性说明为准；通用图表文档中的提案不代表已经实现。

## 领域设计

- [Footprint / Numbers Bars](features/footprint.md)：成交真值、聚合、主动方、LOD、恢复与证据。
- [深度、流动性与成交带视图](features/depth.md)：订单簿投影、序号、重新同步、热力图与回放。

这两份文档补充领域细节；crate 所有权与跨后端执行仍以[架构](Architecture.md)为准。

## 维护约定

当前实现写入 `architecture/`，调用契约写入 `api/`，未来目标保留在 `plan/`。同一规则只保留一个详细说明，其余页面链接过去。更新入口和相对链接，使用简体中文，并保留资源上限、失败/恢复行为与证据局限；完整约定见[贡献指南](development/contributing.md#文档维护)。
