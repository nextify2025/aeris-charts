# Aeris Charts 证据基准测试

该子系统是 Aeris Charts 性能、产物体积与内存声明的事实来源。它通过公共浏览器 API 测量生产版 `@aeristerminal/aeris-charts` 包，并让每个数字都与源码、环境、场景、数据集和原始样本绑定。它不负责优化产品，也不会凭空生成缺乏依据的数值。

## 环境要求

- 仓库所配置的 Rust 工具链，以及用于生产 WASM 构建的 `wasm-pack` 0.15.0。每个工作流都精确安装该版本（`release_gate_guard.mjs` 对 `ci.yml`、`publish.yml` 和三个基准测试工作流强制执行这一点）。wasm-pack 会运行在 `PATH` 中找到的 `wasm-opt`，否则下载其自带的 binaryen `version_117`，因此本地安装的 `wasm-opt` 会悄然改变产物；下文的溯源信息记录了实际运行的是哪一个。
- Node.js 18 或更新版本，以及 Bun 1.4.2（安装依赖并运行包构建）；npm 仅用于 `npm pack`。
- 为 Playwright 安装的 Chromium（`cd examples/web_demo && bunx playwright install chromium`）。
- 对于正式 release 结果，需要在受控 runner 上使用干净检出，并设置 `AERIS_CHARTS_BENCH_ENV_CLASS=official-benchmark-runner` 与稳定的 `AERIS_CHARTS_BENCH_ENV_ID`。

当包或演示的依赖项缺失时，CLI 会运行 `bun install --frozen-lockfile`。浏览器场景复用现有的演示服务器与 Playwright 依赖。不会向使用方发布任何基准测试包。

## 命令

```text
node benchmarks/benchmark.mjs test
node benchmarks/benchmark.mjs smoke
node benchmarks/benchmark.mjs nightly
node benchmarks/benchmark.mjs release
node benchmarks/benchmark.mjs soak --duration-ms 600000
node benchmarks/benchmark.mjs scenario pan-candlestick-100k
node benchmarks/benchmark.mjs size
node benchmarks/benchmark.mjs rebudget <size-result.json> --tradeoff <text> [--lever <text>]... [--headroom-percent 7]
node benchmarks/benchmark.mjs native
node benchmarks/benchmark.mjs compare <baseline.json> <current.json>
node benchmarks/benchmark.mjs report <result.json>
node benchmarks/benchmark.mjs public <official-release-result.json>
node benchmarks/benchmark.mjs baseline <official-release-result.json>
node benchmarks/benchmark.mjs claims
```

`release` 包含已配置的 60 分钟 soak，并拒绝在不干净的工作树上运行。`--duration-ms` 可以缩短本地验证运行，但更改后的时长属于场景配置的一部分，不得当作规范的 release 运行来比较。

## 配置档与权威性

- `smoke` 是共享 CI 上的快速验证。它能发现损坏的场景和明显的回归，但绝不作为公开证据。
- `nightly` 在共享 runner 上收集更广泛的趋势。其绝对耗时不具权威性。
- `release` 是面向受控机器的完整套件，要求工作树干净。只有在 `official-benchmark-runner` 上运行的该配置档才能生成 `benchmark-public.json` 或成为基线。
- `soak` 单独运行长时间负载。规范的 release 时长为 60 分钟；10 分钟的 smoke 或 30 分钟的工程运行需通过 `--duration-ms` 显式选择。

环境分类默认为 `local`；共享工作流设置为 `shared-ci`。请设置稳定的环境 ID，例如 `official-win-nvidia-01`。结果会在适用时捕获 OS、架构、CPU、逻辑核心数、总 RAM、运行时/浏览器、视口、DPR 和 GPU 信息。对于仅体积运行和原生运行，视口与 DPR 为 `null`；未知的 GPU 驱动、物理核心数和刷新率取值同样保持为 `null`。构建溯源信息记录精确的包构建命令或原生构建命令，Rust、wasm-pack、Node、npm 和 esbuild 的版本，Cargo/包锁文件的哈希，以及适用的 wasm-opt 参数。主机名、用户名、主目录、IP 地址、环境变量和本地仓库路径绝不会被序列化。

## 确定性数据与场景

`shared.mjs` 使用带版本的 xorshift32 生成器。生成器版本、无符号种子、起始时间、间隔、起始价格、波动率、点数、系列数和窗格数共同标识一个数据集。生成的行始终满足 `high >= open/close`、`low <= open/close` 以及 `high >= low`。核心规模为 1K、10K、100K、500K 和 1M 根 K 线。数据集的创建发生在计时安装之前。

规范的场景定义位于 `scenarios.json`；它有意保持为一份小型清单，而不是基准测试 DSL。场景 ID 与版本使方法学的变更显式可见。实质性变更会创建新的场景版本，而不是悄悄改写历史。工作负载涵盖启动、历史数据加载、当前 K 线与追加的流式更新、平移/缩放/十字光标输入、生命周期保留、多图表、多系列/多窗格扩展、包含五个系列/100K 行的 Phase 2 通用仪表盘、跨 1/2/4/8/16 个系列及一个/四个窗格的保留式当前 K 线更新，以及 soak 稳定性。保留式场景在记录帧 CPU 百分位数的同时，还记录语义重建次数以及 WebGPU 的分配、写入和上传量。原生证据还会测量：在 10K/100K/1M 根历史数据上的每个代表性指标，与类型化批量等价的 1/10/100/1K/10K 行批量，单一数据源上的 1/4/8/16 个混合指标，以及一个活动数据源在 1/2/4/8/16 个数据源/指标窗格对上的表现。`crosshair-reset-studies-daily-2520`（强制 Canvas2D，release 配置档）加载 2,520 根日 K 线，并带有交易时段 VWAP、VWAP 带和标准枢轴点，共十一个输出，由于每根柱本身就是一个周期，每个输出在每根柱上各绘制一段柱宽的线段，并让十字光标在其上移动。它在数据集配置中记录间隔与研究集合，隔离出周期重置研究的执行器开销，其 `canvas2d_ops`、`frame_cpu_ms` 和长任务样本是批处理 `Segments` 图元的证据。

## 计时与统计

短时长在浏览器中使用 `performance.now()`，在浏览器之外使用 Node 的单调计时设施。墙钟时间仅作为元数据。冷启动每个样本使用全新的浏览器上下文。稳态场景分别独立声明预热运行与测量运行。

原始样本均予保留。`shared.mjs` 是唯一的统计实现：最小值、最大值、算术平均值、p50、p90、p95、p99、总体标准差与样本数。百分位数采用最近秩法：升序排序后选取第 `ceil(p * n)` 个，并以第一个元素为下限。不剔除任何离群值。

`set_data_api_ms` 测量公共的同步类型化数据调用，其中包含校验、引擎安装、帧构建、命令编码/提交以及后端 present 调用。`first_raf_after_*` 结束于下一次浏览器动画帧回调，并被标注为合成器机会，而非光子可见的证明。系统会记录引擎暴露的实际呈现计数器，但由于浏览器在此处不提供可靠的完成时间戳，可见呈现延迟被报告为不受支持。

交互轨迹在该包最上层的叠加层 canvas 上使用 Playwright 的指针与滚轮输入。`raf_frame_interval_ms` 包含显示节奏与浏览器调度；`frame_cpu_ms` 是现有的从引擎到命令编码的测量值。有效 FPS 由观察到的 rAF 回调推导而来，并且始终连同帧时长和刷新率的限制一并给出。

## GPU 与 CPU 测量方法

在 WebGPU 上，仅当现有的 `frame_stats().gpu_ms` 产生已解析的硬件 timestamp-query 样本时，才接受 `gpu_render_pass_ms`。该值覆盖 WebGPU 渲染 pass，并通过异步回读获得。在 Canvas2D 或不具备 `timestamp-query` 的适配器上，它为 `unsupported`，绝不为零，也绝不以 CPU 提交时间代替。

浏览器 CPU 是一个场景期间 Chromium DevTools `Performance.TaskDuration` 的增量。它是对页面主线程任务的测量，不是整机 CPU，也不能完全归因于 Aeris Charts。帧 CPU 来自现有的有界 WASM 遥测记录。

## 内存与生命周期测量方法

内存标签保持区分：

- `wasm_linear_memory_bytes` 是已预留的 WASM 线性内存。它对整个模块是全局的，以 64 KiB 页为单位增长，不会收缩，也不是总 RAM。
- `browser_js_heap_used_bytes` 是经过文档化的 DevTools GC 之后 Chromium 整个页面的 JavaScript 堆。
- `browser_page_memory_*` 在可用时使用 `measureUserAgentSpecificMemory`，是整个页面的内存，并非 Aeris 所拥有内存的精确值。

生命周期场景首先完成并丢弃一次创建/加载/渲染/销毁的预热，使 WASM 初始化和分配器的初始增长发生在基线之前。随后它们重复同一个确定性夹具，在每个已加载的周期记录 WASM 线性内存。由于整页内存采集具有干扰性，smoke 只取一个已加载样本，而 50 周期和 100 周期的配置档分别取五个和十个均匀分布的已加载样本；初始样本和最终样本括住每次运行。结果会记录是否强制了 GC。保留增量是最终整页内存减去预热后的初始基线；每周期保留增量是该值除以已完成的周期数。不受支持的页面内存 API 仍显式标明。多图表与多系列扩展在全新的页面 realm 中加载每一种配置，使不会收缩的 WASM 线性内存高水位线保持可比；夹具创建与具有干扰性的页面内存采集不计入启动计时。

soak 采样会持续更新当前 K 线、周期性追加、移动时间比例尺、移动十字光标，并周期性采样，且不在每帧写文件。轻量的帧、更新延迟和 WASM 内存样本每秒采集一次。干扰性更强的整页内存 API 每分钟采样一次，以免其主导工作负载。当观测数据足够时，结果会报告按每小时归一化的首尾内存增长，以及后半段相对前半段的帧 CPU 退化；否则该派生指标显式标为不受支持。原始的周期性样本保留在结果中。

## 产物体积

体积场景运行发布前使用的同一个生产构建 `bun run build`，并读取 `npm pack --json --dry-run`。各项指标含义明确：

- 仅针对 npm 将要发布的那些文件的 npm tarball 与解包后字节数；
- 生产 JavaScript 的原始/gzip-9/Brotli 字节数；
- 生产优化后 WASM 的原始/gzip-9/Brotli 字节数；
- TypeScript 声明文件的总字节数；
- 由该包现有的 esbuild 依赖构建的最小、典型和完整的压缩（minified）使用方 JavaScript bundle。

使用方 JavaScript bundle 指标明确不包含单独发布的 WASM 资源，其体积单独报告。

只有测量的是经过优化的模块，体积结果才有意义，因此当构建日志显示 wasm-opt 未运行时（在 wasm-pack 无法获取 binaryen 的平台上，它会打印 `Skipping wasm-opt`），该步骤会失败。每条结果都会在 rustc 与 wasm-pack 版本旁记录 `build.cargo_profile`、`build.wasm_opt_args`（读取自 wasm-pack 自身读取的 crate 元数据，而非副本）以及 `build.wasm_opt_version`（`PATH` 上的 `wasm-opt`，否则为 wasm-pack 缓存中最新的一个，再否则为 `null`）。`build.profile` 保持为 `release`，即证据通道，`compare` 会将构建元数据不同的结果视为不兼容。

## 结果、基线、预算与报告

带版本的 JSON 契约是 `schema/result-v1.schema.json`。运行时校验会拒绝缺失的元数据、伪装成成功的失败场景、非有限样本以及不可能出现的负时长。场景失败带有 `status: failed` 与一条错误；公开摘要会排除它们。

本地原始结果是位于 `benchmarks/results/v<version>/<environment-id>/` 之下的不可变文件，并已被 git 忽略。CI 将它们作为产物上传。release 结果应以不可变方式附加到对应的发布版本。`baseline` 仅将干净的正式 release 结果复制到 `benchmarks/baselines/v<version>/<environment-id>.json`，并拒绝覆盖。所选策略是显式的上一个发布版本基线；它绝不会悄悄滚动更新。

比较要求场景版本、生成器版本、种子、数据集配置、点数/系列数/窗格数、稳定的环境 ID、OS、架构、CPU、运行时/浏览器版本、GPU 标识、视口、DPR 以及刷新率元数据全部相同。输出包含基线、当前值、绝对差、百分比差、方向、场景/环境兼容性与状态。预算仅存在于 `budgets.json`。在存在受控基线证据之前，相对耗时阈值保持为空；添加阈值需要同时给出警告与失败百分比，键为 `<scenario>.<metric>.p50`。

阻断性的 `absolute_maximums` 使用相同的键约定。确定性的产物上限不需要机器基线；对机器敏感的通用仪表盘启动/上传上限仅在正式 release 基准测试 runner 上评估，其稳定的环境标识是 release 证据的一部分。每次包含具名场景的基准测试运行都会评估其已配置的最大值；超出、不可用或失败的指标会使进程以非零状态退出。初始的包上限依据提交 `813230b` 上一次干净的生产构建设定，并在其实测输出之上向上取整：

| 指标 | 观测字节数 | 阻断上限 |
| --- | ---: | ---: |
| npm tarball | 960,989 | 1,050,000 |
| npm 解包后 | 2,811,610 | 3,000,000 |
| JavaScript 原始 | 577,227 | 620,000 |
| JavaScript Brotli | 87,948 | 95,000 |
| WASM 原始 | 1,975,672 | 2,100,000 |
| WASM Brotli | 583,569 | 625,000 |

预算策略 v3 记录了在完整的笛卡尔坐标系 API 落地之后，有意进行的 Phase 2 包体积重置。在调整上限之前，已发布的 ESM 构建改为启用压缩；这使 JavaScript 原始体积从 697,316 字节降至 343,161 字节，Brotli 体积从 96,233 字节降至 64,038 字节，因此 JavaScript 的两个原有上限均保持不变。无法再缩减的优化后 WASM 与包容器增长，则以适度的 release 余量纳入：

| Phase 2 指标 | 观测字节数 | 阻断上限 |
| --- | ---: | ---: |
| npm tarball | 1,197,880 | 1,300,000 |
| npm 解包后 | 3,435,987 | 3,700,000 |
| JavaScript 原始 | 343,161 | 620,000 |
| JavaScript Brotli | 64,038 | 95,000 |
| WASM 原始 | 2,812,727 | 3,000,000 |
| WASM Brotli | 761,514 | 810,000 |

这次重置与 Phase 2 由引擎拥有的笛卡尔坐标系接口面相关联（额外的数据通道、参考/刷选/共享提示框 API、热力图变体、持久化以及 WASM 绑定）。未来的增长再次在 v3 上限处被阻断，而不是继承一个没有边界的例外。

预算策略 v4 记录了在交易工作站接口面落地之后的 Phase 3 包体积重置：订单流足迹图与成交流、深度热力图与流动性回放、含非时间柱的交易时段回放、分布工作流、扩充后的指标目录以及新增的图表类型。在调整上限之前，已使用 `--converge` 以及剥离 producer/调试信息重新运行 release 的 `wasm-opt -Oz` 输出；二者均未缩小该模块（分别为 3,826,918 与 3,833,897 字节）。JavaScript 仍远在其未变的上限之内。剩余的增长来自已编译的引擎代码，因此 WASM 与包容器的上限沿用与 v3 相同的约 7% release 余量：

| Phase 3 指标 | 观测字节数 | 阻断上限 |
| --- | ---: | ---: |
| npm tarball | 1,549,907 | 1,650,000 |
| npm 解包后 | 4,628,626 | 4,950,000 |
| JavaScript 原始 | 372,974 | 620,000 |
| JavaScript Brotli | 66,155 | 95,000 |
| WASM 原始 | 3,827,699 | 4,100,000 |
| WASM Brotli | 977,494 | 1,050,000 |

在未剥离的模块中测得的最大可缩减份额是 serde JSON（反）序列化的单态化（约占优化前代码的五分之一），其中以内部带标签的 `IndicatorKind` 枚举为首（其反序列化器现已改为非内联，见下文）。在下文的策略 v5 之前，未来的增长一直在 v4 上限处被阻断。

预算策略 v5 是在 B1-B8 K 线能力（交易所时间与交易时段槽位、价格刻度阶梯、为每个带文本的工具提供文本编辑的 B8 绘图目录、多日历叠加层、由 Tick 构建的 K 线与重采样、收盘时间标签）以及合入上游后续工作之后的重置，其中 IANA 时区表在过滤到 98 个 TradingView 时区后，增加约 350 KB 原始体积与 41 KB Brotli 体积（完整数据库约为 914 KB 与 84 KB）。这些字节数是在 GitHub runner 上使用固定的 wasm-pack 0.15.0 及其自带的 `wasm-opt` 测得的；所发现的唯一无损杠杆（`IndicatorKind` 反序列化器）已先行发布，而模块仍然超出的那些上限，采用相同的 7% release 余量，并向上取整到 10,000 字节。JavaScript 仍在其未变的上限之内。其余每一项缩减都属于 opt-level 变更，会以帧时间为代价（下文给出其代价），并等待产品决策；证据记录在 `budgets.json` 的 `rationale` 中。

| Phase 4 指标 | 观测字节数 | 阻断上限 |
| --- | ---: | ---: |
| npm tarball | 1,969,915 | 2,110,000 |
| npm 解包后 | 5,953,058 | 6,370,000 |
| JavaScript 原始 | 415,362 | 620,000 |
| JavaScript Brotli | 74,089 | 95,000 |
| WASM 原始 | 5,041,592 | 5,400,000 |
| WASM Brotli | 1,237,844 | 1,330,000 |

在下文的策略 v6 之前，未来的增长一直在 v5 上限处被阻断。

预算策略 v6 是在借鉴 Vela 的引擎批次之后的重置（带有对已绘制最后一根柱的显示覆盖的实时柱缓动、基线参考线与 `baseline_mode`、十字光标 `shadeRight` 遮罩，以及带有槽位映射、聚类、停留提示框、输入结果、隐藏分组持久化与 WASM/TypeScript 接口面的时间线标记带）。相对于批次基础提交 `de7d571`，模块增长了 92,557 原始字节（+1.7%）与 24,284 Brotli 字节（+1.9%），使用同一个 wasm-pack 0.15.0 及其自带的 `wasm-opt` 117 测得；该基础提交本身已经比 v5 上限低 20,453 原始字节，因为上游后续工作已消耗了 v5 的余量。增长来自这四项能力编译后的引擎代码（时间线标记类型及其 JSON 接口面、按系列的选项输出器、缓动状态与显示投影），没有单一可缩减的热点；下文定价的各无损杠杆已重新核对，没有一个适用于本批次，其余每一项缩减都是以帧时间为代价的 opt-level 变更。只有本次运行超出的两个上限移动（WASM 原始与 npm 解包后），移到观测 p50 加 7% 再向上取整到 10,000 字节；JavaScript 与 WASM Brotli 保持在各自未变的上限之内。npm tarball 比其未变的上限低 2,421 字节，因此发布 runner 上不同的压缩器可能使其超出；那将是一次需要有意重新设定预算的测量，而不是缺陷。证据记录在 `budgets.json` 的 `rationale` 中。

| Phase 5 指标 | 观测字节数 | 阻断上限 |
| --- | ---: | ---: |
| npm tarball | 2,107,579 | 2,110,000 |
| npm 解包后 | 6,392,809 | 6,850,000 |
| JavaScript 原始 | 408,085 | 620,000 |
| JavaScript Brotli | 71,326 | 95,000 |
| WASM 原始 | 5,472,104 | 5,860,000 |
| WASM Brotli | 1,320,160 | 1,330,000 |

未来的增长在 v6 上限处被阻断。

### WASM 体积杠杆与重新基线

B1-B8 落地之后，优化后的 WASM 不再满足 v4 上限。在移动任何上限之前，已先测量字节数，并量化了各无损杠杆的代价。下文所有数字均来自同一台机器（rustc 1.98.1、wasm-pack 0.15.0、wasm-bindgen 0.2.127、`PATH` 上来自 `binaryen@117.0.0` npm 包的 `wasm-opt` version_117、Chromium 141 无头模式、强制 Canvas2D、一个共享的四核沙箱），基于基础提交 `78d7d59`。构建路径也曾手动运行（`cargo build -p aeris_charts_wasm --release --target wasm32-unknown-unknown`、`wasm-bindgen --target web --out-name aeris_charts_wasm`、带 crate 元数据标志的 `wasm-opt`），产出的模块与 `wasm-pack build` 字节完全一致。它们是工程证据，既不是 CI runner 上的性能声明，也不是公开的性能声明。

| 包指标 | `78d7d59` | `IndicatorKind` serde 非内联后 | v4 上限 |
| --- | ---: | ---: | ---: |
| npm tarball | 1,876,748 | 1,860,741 | 1,650,000 |
| npm 解包后 | 5,636,275 | 5,503,213 | 4,950,000 |
| JavaScript 原始 | 412,622 | 412,622 | 620,000 |
| JavaScript Brotli | 73,630 | 73,635 | 95,000 |
| WASM 原始 | 4,739,804 | 4,606,742 | 4,100,000 |
| WASM gzip-9 | 1,663,479 | 1,647,743 | - |
| WASM Brotli | 1,175,083 | 1,172,054 | 1,050,000 |

（tarball 与解包后两列均已排除 `dist/react.js.map`，同一变更不再发布该文件：对应 8,165 字节 tarball 与 27,149 字节解包后体积。测量本身不会改变上限：请在最终代码上使用下文的 `rebudget` 推导。）

该模块 93% 为代码（4.40 MB），6% 为数据（0.28 MB，其中十字光标蒙版为 100,368 字节），导入与导出不足 1%。代码是长尾分布，而非单一热点。对同一模块保留名称的 `-Oz -g` 构建运行 Twiggy，其归因（占代码的百分比）为：`aeris_charts_engine` 31%，wasm crate 11% 外加其 wasm-bindgen 导出垫片，为这些 crate 实例化的 libcore 与 alloc 泛型 25%（仅 `slice::sort` 就占 6%，涉及约十五种元素类型），由 serde 派生的反序列化在 engine 与 wasm crate 中约占 25%（`serde_json` 本身 3%），`aeris_charts_core` 3%，indicators 2%，render、render_wgpu 与 wgpu 各约 1%。最大的函数，即每帧运行的 `ChartInner::render_inner`，占代码的 6.6%；最大的十个函数占 20%。依赖项原本就已处于 `opt-level = "z"`。

| 杠杆（未列出的工作区 crate 保持 `opt-level` 3） | WASM 原始 | Brotli | 原始体积相对当前 | 平移 / 流式 帧 CPU p50，相对当前的配对比值 |
| --- | ---: | ---: | ---: | --- |
| 当前 | 4,739,804 | 1,175,083 | - | 1.00 / 1.00 |
| `IndicatorKind` serde 非内联（已发布） | 4,606,742 | 1,172,054 | -2.8% | 1.00 / 1.06（平移在 7 轮中有 3 轮更慢，流式在 7 轮中有 6 轮更慢：对于一项不在帧路径上的变更，该流式数字属于这台机器的噪声下限） |
| `wasm-opt` 使用 binaryen 132 而非 117 | 4,717,722 | 1,174,150 | -0.5% | 未计时 |
| `wasm-opt --converge` | 4,738,993 | 1,173,017 | -0.02% | 未计时 |
| 不使用 `+simd128` | 4,803,949 | 1,179,862 | +1.4% | 未计时（SIMD 不带来字节开销） |
| engine `s` | 4,268,389 | 1,115,992 | -10.0% | 0.99-1.03 / 1.01-1.06 |
| engine `z` | 3,895,650 | 1,057,463 | -17.8% | 1.06-1.08 / 1.00-1.07；缩放 1.18，十字光标 1.11 |
| wasm crate `z` | 4,515,650 | 1,172,435 | -4.7% | 0.99 / 1.17 |
| `aeris_charts_core` `z` | 4,656,252 | 1,170,339 | -1.8% | 未单独计时 |
| indicators `z` | 4,707,715 | 1,171,271 | -0.7% | 未单独计时 |
| render 与 render_wgpu `z` | 4,718,680 | 1,172,342 | -0.4% | 未单独计时 |
| engine + wasm crate `z` | 3,464,533 | 987,566 | -26.9% | 1.09 / 1.18 |
| engine + wasm + core + indicators `z` | 3,292,048 | 944,719 | -30.5% | 1.42 / 1.19 |
| 每个工作区 crate `s` | 3,741,299 | 1,025,898 | -21.1% | 一轮筛选：1.09 / 1.14 |
| 每个工作区 crate `z` | 3,250,690 | 933,355 | -31.4% | 1.49 / 1.30 |

只有第一行已发布：它是唯一一个效果纯粹体现为代码体积的杠杆（约 110 KB 的内部带标签 `IndicatorKind` 反序列化器存在两份，一份用于 `from_value`，一份用于结构体字段，而 Brotli 已经掩盖了其中大部分重复），位于冷路径上，且渲染出的帧字节完全一致。每个 `opt-level` 行都是以帧时间换取字节数，并在产品决策作出之前保持未发布。

计时行是交错 A/B 运行中各轮配对比值的中位数（每个变体至少七次交替运行，每次运行使用全新的浏览器，场景为 `pan-candlestick-100k`、`stream-current-candle-60hz`、`zoom-candlestick-100k`、`crosshair-candlestick-100k`）；区间为各独立组之间的范围。这台机器上的噪声下限很大：当前构建自身运行间的 p50 离散度，平移约为 15-22%，流式约为 16-54%，而 100k 仪表盘的 p50（每次运行五个样本）波动约 30%，因此任何一行中的仪表盘差异都无法区分。engine `s` 的十五轮交错平移与流式运行给出的 p50 比值分别为 1.03（平移，15 轮中有 10 轮更慢）和 1.01（流式，15 轮中有 9 轮），合并后的 p95 为 +5% 与 -3%：不能排除平移上最多几个百分点的代价，这正是未更改 release 配置档的原因。当前构建与 engine `s` 构建渲染出的帧，在五次确定性的 Canvas2D 演示画面捕获中字节完全一致。在 engine 与 wasm crate 构建的基础上，再将 `aeris_charts_core` 与 indicators 设为 `z`，使平移从 1.09 升至 1.42；这两个 crate 之间的占比划分未单独计时，因此无法确定其中哪一个承载每帧循环；`z` 级别的 wasm crate 在流式上显示 +17%，且离散范围很宽（每轮 0.76-1.37）。

若要在不编辑 release 配置档的情况下复现某个变体：使用 `--config 'profile.release.package.<crate>.opt-level="z"'` 构建（Cargo 环境变量无法表达按包的覆盖），如上所述运行 `wasm-bindgen` 与 `wasm-opt`，并用该变体的 `aeris_charts_wasm.js` 重新打包 `dist/index.js`（胶水代码按每次构建的索引为闭包垫片命名，因此来自其他构建的胶水文件不匹配）。

在有意的体积增长之后重新基线时，在最终代码上运行 `node benchmarks/benchmark.mjs size`（它会将结果 JSON 写入 `benchmarks/results/` 之下，并在上限被超出时以非零状态退出），然后执行

```text
node benchmarks/benchmark.mjs rebudget <that result.json> --tradeoff "<the product capability that added the bytes>" --lever "<lever applied and its measured effect>"
```

它会打印拟议的 `budgets.json` 而不写入文件：只有本次运行所超出的上限才会移动，移到观测 p50 加 7% 再向上取整到 10,000 字节（Phase 2 重置的余量为 6.4-8.5%），`policy_version` 递增，并追加一条 `rationale` 条目，其中包含提交、观测字节数、被提高的上限、工具链（rustc、wasm-pack、wasm-opt 版本、Cargo profile、wasm-opt 标志）、所用杠杆与权衡。`budgets.json` 会忽略其不评估的键，因此证据与数字并存。审阅 diff，将其与导致增长的代码一并提交，并确认 `ci.yml`、`metrics-smoke.yml` 以及 nightly 和 release 工作流在此之下均能通过。

Phase 2 为 `general-dashboard-100k` 增加了阻断 release 的最大值：从启动到其后第一个 rAF 的 p50 必须不超过 2,000 ms，首帧 WebGPU 顶点上传量必须不超过 96 MiB。这些是针对宿主灾难性回归的护栏，而不是跨机器的性能声明。

`crosshair-reset-studies-daily-2520` 增加了一个阻断 release 的最大值：每个十字光标帧 4,500 次 Canvas2D 绘制操作（p50）。在周期重置研究的单柱线段被批处理为每个输出一个 `Segments` 图元之前，该场景在无头 Chromium 141 中（强制 Canvas2D，1280x720，软件光栅化）每帧绘制 31,116 次操作；不含研究的同一图表绘制 3,822 次，启用批处理后为 3,884 次。该上限略高于批处理后的计数，为平台标签差异留出余地，并远低于未批处理的计数，因此一旦回到每根柱一次描边，就会使门禁失败。它只约束操作数量：测得的帧 CPU（`frame_cpu_ms` p50 在此前为 63.8 ms，此后为 11.8 ms，不含研究时为 5.0 ms）依赖机器，仍仅作报告。

这些字节数是可复现的文件系统/压缩证据，不是正式的墙钟基准测试，也不是公开的性能声明。有意的体积增长必须说明产品上的权衡并更新中央上限；不得绕过评估器。

`report` 在原始结果旁生成一份人类可读的 Markdown 产物。`public` 生成稳定的 `benchmark-public.json`，其中仅包含来自干净的正式 release 的、实测的 `public_candidate` 指标。追溯链路是：网站字段 → 公开摘要 → 不可变的原始结果 → 原始样本 → 带版本的场景 → 确定性数据集/环境 → 提交。

## CI 与局限

PR 的 smoke 在共享 CI 上运行 harness 测试、生产产物体积以及一个简短的浏览器工作负载。Nightly 运行范围更广、但不具权威性的场景。release 工作流面向未来的自托管 Windows runner（标签为 `benchmark`），产出原始 JSON、可读报告与公开摘要。凭据与 runner 的配置有意放在本仓库之外。

`native` 命令为无头的 Rust 数据写入、保留式帧构建以及当前 K 线替换单独写出一份结果。原生诊断结果绝不会与浏览器产品结果或公开摘要混在一起。

竞品比较在此有意不予实现。未来的比较必须使用独立的结果命名空间，固定每个库和浏览器的版本，披露后端与功能配置，使用等价的可见图表功能和数据集，应用相同的预热/统计/环境规则，并将方法学与数字一并发布。现有的演示依赖项不被视为基准测试的竞品。

目前无法以可信的语义测量：光子可见的呈现完成、单独的首次 `queue.submit` 时间戳、仅 GPU 上传的时长、GPU 资源字节归因、浏览器 GPU 驱动版本、物理核心数以及原生进程 RSS。它们保持为不受支持或 `null`；不会推断任何替代数字。现有的交互与 GPUI 场景计划示例仍然是有用的内部诊断。
