# 验证门禁与平台证据

[文档导航](../README.md) · [架构总览](../Architecture.md)

执行命令从仓库根目录开始，除非代码块显式切换目录。仅文档变更检查 diff、相对链接/锚点、源码与架构表述及中文规范；不要求运行未受影响的运行时门禁。

- [正确性与真实调用路径](#正确性与真实调用路径)
- [标准门禁](#标准门禁)
- [固定工具链与严格预算](#固定工具链与严格预算)
- [浏览器与 Linux 呈现](#浏览器与-linux-呈现)
- [GPUI 一致性与回放](#gpui-一致性与回放)
  - [gpui-fast 证据线](#gpui-fast-证据线)
- [发布与诊断的边界](#发布与诊断的边界)

## 正确性与真实调用路径

对于相同的状态、视口和设备缩放比例，图表数学计算必须是确定的。在输入边界校验格式错误的数据。保留空白数据行、时间顺序、逻辑范围、图元顺序和显式的预热间隙。

对几何、磁吸、比例尺、交互或执行的变更，要求提供单元测试、帧契约测试、黄金图像、绘制流一致性、回放稳定性、浏览器测试和 release 性能证据中范围最窄的相关组合。仅凭特定后端的截图，不能证明共享引擎的正确性。

交互行为通过每条真实的输入路径来验证，而不是通过其下层的机制。控制器的场景模块（`chart_input/{chrome,drawing,lifecycle,motion,trading}_tests.rs`，与其自身的 `tests` 并列）通过 `ChartEngine::input_*` 并配合显式时间戳来驱动每个行为（准备工作以及诸如 `resolve_trading_intent` 之类的宿主答复使用公共 API），因此动能滑行和交易提示框停留在没有时钟的情况下也是确定的。GPUI 适配器的测试（`aeris_charts_render_gpui/src/input/tests.rs`）把 GPUI 事件值送入 `GpuiChartInput`，并在孪生引擎上将其滚轮归一化与浏览器的滚轮归一化进行比较；在 GPUI 的测试执行器上，它们还覆盖刷新契约：执行器时钟、notify、每个截止时间一个唤醒（保留、替换、丢弃、随适配器取消）、触发的唤醒到达其截止时间、与该唤醒共享的倒计时整秒，以及减弱动效会停止的脉冲时钟。`gpui_probe` 示例的 `window_input_tests` 在 GPUI 的无头 `TestPlatform`（仅用于开发的 `gpui` 依赖的 `test-support` 特性）上打开真实的探针宿主，并把模拟的鼠标、滚轮和按键事件依次派发经过其监听器表、适配器、引擎、prepaint 和 paint；它们在全部三种操作系统上的原生 GPUI 作业中运行。它们的宿主以 `.cached()` 嵌入图表，因此未被 notify 的图表会像在 gpui-fast 的保留模式下那样重放其最后一帧。在该宿主上（注明之处也在未缓存的宿主上），它们断言：指针运动、滚轮和按键各自让图表以重建的帧绘制一次；没有 `refresh` 的变更保持未绘制；执行器时钟越过截止时间后，提示框停留无需进一步输入即会重绘（被替换的唤醒绝不触发）；按住的按键、惯性滑行和脉冲在执行器时钟上每绘制一帧请求一帧，并在结束时停止，若最后一步改变了状态则恰好再多一帧；按住的按键在其按键抬起之前若键盘焦点移开或窗口失活则结束；减弱动效移除脉冲及其帧；显示中的倒计时无需输入即每秒重绘一次，并在没有倒计时行显示时停止；有限探针每次绘制排入一帧；平移会跟随移出图表元素、进入宿主边距的指针（缓存与未缓存的宿主上均如此），并在指针回到同一 x 时得到同一视图；按键（图表忽略的键和它处理的缩放键）不会清除静止指针下的十字光标，而之后指针离开仍会清除它；窗口失活会放弃进行中的按下：绘图拖动回滚，平移停在原处，按键仍按住的后续移动不再改变任何东西。同一套件也在 [gpui-fast 证据线](#gpui-fast-证据线)中针对 gpui-fast 运行。浏览器交互由 Playwright spec 覆盖，这些 spec 针对已发布的包构建，驱动真实的 `page.mouse`/`page.keyboard` 输入和 CDP 触摸（`gesture-cancellation`、`interaction-gates`、`touch-input` 以及各功能专属的 spec）。

## 标准门禁

标准门禁与 CI 一致（`.github/workflows/ci.yml`，顺序相同）。Bun 1.4.2 安装 npm 依赖（`packages/charts` 和 `examples/web_demo` 中的 `bun.lock`），并运行包脚本和 Playwright；仍安装 Node 24，因为 Playwright 和仓库的 `node` 脚本运行在它之上。npm 仅用于其定义了已发布产物的场合：用于体积预算和打包冒烟测试的 `npm pack`，以及 `publish.yml` 中的 `npm publish`。

```text
node packages/charts/scripts/namespace_guard.mjs
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p aeris_charts_wasm --target wasm32-unknown-unknown --locked -- -D warnings
cargo test --workspace --locked
AERIS_CHARTS_PERF_STRICT=1 cargo run -p aeris_charts_native --example perf_gate --release

cd packages/charts
bun install --frozen-lockfile
bun run lint
bun run build
node ../../benchmarks/benchmark.mjs size   # CI 从仓库根目录执行
bun run typecheck
bun run check:api
bun run check:release-gates
bun run test:pack
```

有意为之的公共 API 变更，会先用 `bun run update:api` 重新生成快照（这是写入步骤，不是门禁），再运行 `check:api`。CI 还要求通过可移植浏览器套件（在 `examples/web_demo` 中，先在该目录执行 `bun install --frozen-lockfile && bun run build`，再执行 `AERIS_CHARTS_PORTABLE_BROWSER=1 bunx playwright test`）和原生 GPUI 测试（`cargo test -p aeris_charts_render_gpui --features gpui-backend --all-targets --locked`），并在 Linux 上对 GPUI 后端及其测试做 lint（`cargo clippy -p aeris_charts_render_gpui --features gpui-backend --all-targets --locked -- -D warnings`）。

## 固定工具链与严格预算

CI、标签发布工作流和基准测试工作流使用 `cargo install wasm-pack --locked --version 0.15.0` 安装 `wasm-pack` 0.15.0：其捆绑的 `wasm-opt` 决定了发布的 WASM 字节，进而决定包体积预算。当其中任何一个工作流未固定版本或使用其他版本安装它时，`bun run check:release-gates` 会失败。

Rust 工具链同样是确切的发布版本：`rust-toolchain.toml` 指定了它（1.99.0），每个工作流都通过其 `toolchain:` 输入安装该版本，因此新的 stable Rust 无法改变 CI 编译或 lint 所用的内容（浮动的 `stable` 曾在一棵未改动的代码树上新增了一条 Clippy lint）。当某个工作流与该文件不一致时，`bun run check:release-gates` 会失败。更换该固定版本是一次有意为之的提交，它同时更改两处，并重新运行体积和性能预算。

`perf_gate` 会针对每个目标输出 PASS/FAIL，并且仅当 `AERIS_CHARTS_PERF_STRICT=1` 时（必须恰为 `1`，这与浏览器性能 spec 采用的解析方式相同；未设置或为 `0` 时仍仅作报告）才会在失败时以非零状态退出，因此上方的本地门禁命令行保留该变量以与 CI 保持一致。逐帧目标是在一个预热帧之后取单窗口均值，因此应在其他方面空闲的机器上运行该门禁：并发的构建或浏览器运行可能使一个在空闲运行下能通过的预算失败。

对下文非阻塞墙钟策略的已知例外：两个 `ring-source.spec.mjs` 断言度量墙钟行为（达到的生产者速率，以及“frame cost includes a sustained 50,000 rows/s drain”所测的 8 ms 帧成本中位数），并在必需的可移植套件中运行。确定性的环形缓冲区契约（每个 Tick 零次引擎调用、每帧排空、溢出报告）无论如何都是阻塞的。

## 浏览器与 Linux 呈现

本地浏览器运行需要 Playwright 固定的浏览器构建（对于固定的 Playwright 1.63，为 Chrome for Testing 153）。对于 WebGPU spec，Chromium 的托管方式与其版本同样重要。在使用仓库启动参数（SwiftShader WebGPU adapter）、无 GPU 的 Linux 机器上，以 Chromium 141.0.7390.37 测得：无头 Chromium 能获得 adapter 和 device，但无法呈现 WebGPU 帧（`chrome://gpu` 将 `gpu_compositing` 报告为 `disabled_software`，将 `webgpu` 报告为 `unavailable_software`，device 在第一个呈现的帧上丢失，演示页记录 `WebGPU fallback ... reason=device_lost` 并在 Canvas2D 上运行），因此每个 WebGPU 对 Canvas2D 的 spec 都在比较像素之前，就在后端断言处失败。

在虚拟显示器下使用有头 Chromium 可以正确呈现（`gpu_compositing` 和 `webgpu` 均已启用，通过系统 Mesa llvmpipe GL 之上的 ANGLE），并且演示页报告后端为 `webgpu`，图表完整。在另外一些参数组合下，无头运行会报告后端为 `webgpu`，但画布是空白的，因此 WebGPU 检查还必须查看像素，而不能只看所报告的后端。因此，像素一致性 spec 在虚拟显示器下以有头模式运行（`xvfb-run`）。在该环境下，`prim-text`、`drawings`、`primitives`、`series-primitives`、`custom-series`、`builtin-plugins` 和 `canvas-primitives` 在默认字体栈、随附的 Roboto 和未启用 hinting 的渲染下均能通过，七个绘图族一致性 spec 在默认字体栈下通过，因此演示夹具不固定字体。

要在没有 GPU 的 Linux 机器上运行仓库的 WebGPU spec，在已安装 Playwright 固定的浏览器构建的情况下，从 `examples/web_demo` 在虚拟显示器上以有头模式托管 Chromium：

```text
xvfb-run -a -s "-screen 0 1920x1080x24" bunx playwright test --headed <specs>
```

在上游 B7/B8 同步之后以这种方式测得（Chromium 141，可移植套件），每个绘图 spec 均通过：`drawings.spec.mjs`、七个绘图族 spec（`drawings-lines`、`drawings-channels`、`drawings-fibonacci`、`drawings-pitchforks-gann`、`drawings-patterns-waves-cycles`、`drawings-projection-annotations` 和 `drawings-shapes`，每个都带有其 WebGPU 对 Canvas2D 像素完全一致的测试）、`drawings-text-editing`、`measure-tools`、`resampling` 和 `tick-bars-resampling`，`indicators.spec.mjs` 的 WebGPU 测试同样通过。有九个测试在这类机器上对环境敏感，并在该机器上失败：`backend-parity.spec.mjs` 中的四个（`translucent zig-zag joins match Canvas2D on WebGPU`、`oversized text run remains visible on the WebGPU chart`，以及两个 `measure areas are pixel-identical` 测试，它们此前有的运行通过、有的运行失败），`alerts.spec.mjs` 中的两个 `crosshair action visibly matches the circular SVG` 测试，`worker frames continue while the main thread is blocked for 500 ms`（`offscreen-worker.spec.mjs`），`appending 1M points in batches allocates no per-point JS objects`（`update-typed.spec.mjs`），以及 `interaction models run engine-side with canonical behavior`，后者在 CI 的 60 秒超时下通过。以有头模式运行时，`backend-parity.spec.mjs` 的三个 `reference ...` 测试（标记为 `@machine`，因此不在可移植套件之内）超出其时间轴上限，因为这些上限是在 Windows 上校准的，而该机器只有 DejaVu 字体。（这些测量使用 Chromium 141，因为那是该机器上已安装的构建，通过一份设置了 `executablePath`、其余部分复用 `playwright.config.mjs` 的本地 Playwright 配置来运行。）

在 Linux 上失败的那个像素 spec，`last-value-cluster`（“crosshair price and time glyphs stay centered”），失败的原因在于它的探针，而不是标签位置。共享的坐标轴构建器依据宿主对稳定样本 `Apr0` 的墨迹度量（从大写字母顶部到下伸部底部）来定位十字光标时间文本，绝不依据标签自身的字形；引擎测试 `crosshair_time_text_is_placed_by_the_stable_sample_not_its_own_ink` 固定了这一契约，因此对每个月份名称和每种字体，文本都位于条带内相同的偏移处。在 DPR 为 1、1.25 和 2 时，对 DejaVu Serif、Liberation Sans、Liberation Serif 和 FreeSans，绘制出的 `Apr0` 墨迹落在条带文本中心的 0.65 px 之内，WebGPU 与 Canvas2D 均如此。先前的探针以纯白来度量日历标签的墨迹，因此其结果取决于月份名称（只有部分名称带有下伸部），也取决于细的下伸部笔画在宿主的光栅化器上能否达到纯白：在 Linux 上，六个月份名称的读数各为 0 到 1 px，而要求为 1.5-4 px，且无论怎样选择字体或 hinting 都无法改变这一点。

该 spec 现在通过 `localization.time_formatter` 绘制 `Apr0`，在两个后端上以半覆盖度度量墨迹，要求其落在条带文本中心的一个设备像素之内，并且仍然检查日历标签保留其刻度空间和内边距。一个设备像素的容差涵盖的是半覆盖度墨迹框的光栅取整，而不是字体校准：宿主的修正量就是样本自身测得的墨迹（`logical_midpoint_correction`，相对于 `middle` 基线的 `(ascent - descent) / (2 * dpr)`），因此无论宿主解析出哪种字体，样本的墨迹按构造都居中于条带文本中心。

## GPUI 一致性与回放

对于浏览器行为、渲染、交互、打包或一致性方面的变更，运行 Playwright。对于 GPUI 执行器变更，运行 GPUI 一致性与回放检查。`pixel_parity` 测试框架会先写入 `results.json` 和图像，然后在以下情形退出且状态非零：某个门禁失败、某次捕获返回的尺寸与夹具尺寸不符、`results.json` 无法写入，或运行超过其墙钟截止时间（示例中的 `DEADLINE`，为 120 s，而正常的 Linux 运行耗时 13.7 到 13.9 s；没有 X 显示时 GPUI 只绘制一次，此后不再绘制，因此是该截止时间终止那次运行）。某个夹具的图像在其捕获之前被删除，捕获失败的行不携带 GPUI 或差异哈希，因此 `results.json` 绝不会保留更早一次运行的证据。在 Windows 上（`native-gpui` CI 作业在该平台上将其作为阻塞步骤运行，捕获取自 DWM 窗口），其门禁是官方窗口限值：填充的整数矩形和彩色图像夹具与原生渲染完全一致，十字光标图标图像和缩放后的彩色图像至多相差一个通道值的混合舍入，并且在半透明描边连接处 7 CSS px 范围内，没有任何通道与参考结果相差超过 32（这是邻域最大值，而不是像素占比，因此由 `check_joins` 来约束）；其他夹具仅作报告。对图标源或掩码的变更还要运行 `node examples/web_demo/build_crosshair_icon.mjs --check`。

在仓库根目录下，无头 Linux 运行只需一条命令。它需要 `xvfb` 和 `mesa-vulkan-drivers`，并且虚拟屏幕必须大于测试框架的窗口，因为 X 服务器会把窗口限制在其屏幕范围内：

```text
env -u WAYLAND_DISPLAY GPUI_X11_SCALE_FACTOR=1 \
  VK_ICD_FILENAMES="$(ls /usr/share/vulkan/icd.d/lvp_icd*.json | head -n 1)" \
  xvfb-run -a -s "-screen 0 2560x1600x24" \
  cargo run -p aeris_charts_render_gpui --features gpui-backend --example pixel_parity
```

Linux 门禁（`examples/pixel_parity.rs` 中的 `GATES` 与 `ALIGNED`，其文档注释记录了每个夹具的全部测量值与限值）是针对 dev profile 下的 Mesa lavapipe 25.2.8（LLVM 20.1.2）校准的：在缩放系数 1.0 与 1.5 下各连续运行三次，所得计数与 GPUI 图像哈希均完全相同（在 1.0 下以 `LP_NUM_THREADS=1` 运行一次，结果同样一致）。每个门禁限定一个夹具中与原生参考相差超过某一通道差值的像素所占的比例：在成因允许时要求精确一致，否则取缩放系数 1.0 的测量值再加 25% 的余量，以适应不同的 Mesa 或 LLVM 构建，这不是噪声容限，因为不存在需要容纳的运行间噪声。像素占比对位置而言是较弱的检验，因此抗锯齿夹具还必须满足对齐要求：GPUI 的图像与未平移的参考之间的距离，必须小于它与沿 x 或 y 方向（任一方向）平移一个像素的参考之间的距离。故意注入的回归会使门禁失败：将整个 GPUI 层平移一个像素，会使 crisp、icon、translucent、join 与 image 门禁失败，沿任一方向平移一个像素，会使每个抗锯齿夹具的对齐检查失败；但它不再使 gradients 门禁失败，该门禁的残差主要来自面积填充的边缘毛边。

这些填充边缘门禁（gradients、`opaque_aa`、`tessellated`）在覆盖毛边从上游合并之后重新做了测量：该毛边是为 1x 路径 pass 构建的，而 lavapipe 会解析 4x MSAA，边界像素会同时受到两者的作用，因此填充边缘比参考重约半个像素，三个限值比之前宽松数倍，对齐余量也更小（具体数值见 `GATES` 与 `ALIGNED` 的文档注释）。因此，带 MSAA 的 surface 绘制出的填充边缘会比原生参考重出该幅度。测试框架会在每个夹具的图元之下绘制该夹具声明的背景，因为从上游加入的夹具没有绘制背景，而 Linux X11 窗口会把未绘制的像素捕获为透明黑色。text 夹具会被报告，但其残差不设上限，因为 GPUI 与原生参考绘制的是不同的字体（fontconfig 与 `fontdb` 对“sans-serif”的解析结果不同），所以它度量的是宿主的字体。

一次通过证明：Prim 流的坐标、颜色、绘制顺序与混合运算原样到达了真实的 GPUI 窗口，并且抗锯齿边缘保持在已测得的范围之内。它不证明裁剪：测试框架通过 `GpuiChartRenderer::paint_prims` 绘制每个夹具，这是一个没有帧、也没有裁剪的裸 Prim 层，因此只有下文帧级的窗格矩阵（`gpui_pane_capture`，它通过 `paint_frame` 绘制）才会比较裁剪。它同样不证明硬件行为：lavapipe 的光栅化规则、它对 GPUI 路径 pass MSAA 的解析（取 surface 格式所支持的 4、2 或 1 个采样中的最大值）、gamma、GPUI 在此技术栈上绘制的 LCD 子像素文本，以及字形光栅化，都可能与 DWM/WARP 及真实 GPU 不同。CI 在 `ubuntu-latest` 上以非阻塞的 `gpui-pixel-parity` 作业运行该测试框架，并上传 `results.json` 与 PNG 文件。该作业的非阻塞性来自作业与测试框架步骤上的 `continue-on-error`，并且该步骤设有 10 分钟超时，因此门禁失败、挂起、作业超时或 apt 偶发失败都不会让 `ci.yml` 的运行变红（标签发布工作流要求发布提交对应的 `ci.yml` 运行为绿色）。当同一提交上连续五次 runner 运行都通过每一道门禁、且 `gpui_adapter` 与 `*_rgba_sha256` 的值保持一致，并且 runner 的 Mesa 构建要么与校准构建一致、要么限值已根据 runner 自身的测量重新推导且 Mesa 版本已被固定或其漂移受到监控时，它才会成为必需检查。升级为必需检查时移除两处 `continue-on-error` 标志即可；其他无需改变，因为标签发布工作流要求整个 `ci.yml` 运行成功。

GPUI 与 WebGPU 对比矩阵（`examples/web_demo/tests/gpui-webgpu-matrix.spec.mjs`，通过 `AERIS_CHARTS_RUN_GPUI_WEBGPU_MATRIX=1` 选择启用，通常经由 `bun run test:gpui-webgpu` 运行）在 Linux 上也会运行其四个浅色基础用例，以有头模式在 `xvfb-run -a -s "-screen 0 2560x1600x24"` 下运行，并使用与上文相同的 `VK_ICD_FILENAMES`：`gpui_pane_capture` 读取 GPUI 的 X11 窗口（该 spec 会把 `GPUI_X11_SCALE_FACTOR` 设为夹具的像素比 1.5，且虚拟屏幕必须大于 1851x1047 的窗格），Playwright 则对已呈现的 WebGPU 帧截图。测量使用 Chromium 141.0.7390.37（并非固定版本的构建）及其 SwiftShader WebGPU 适配器，对照 lavapipe，并以 dev profile 的 `gpui_pane_capture` 代替该 spec 中的 `cargo run --release`；连续三次运行对全部四个用例都给出了逐字节一致的输出（0 个像素不同），且两侧的哈希都保持稳定。

已批准的 WebGPU 哈希是 Windows 哈希，七个哈希在 Linux 上均未匹配，因此 Linux 在报告中记录其 WebGPU 哈希，仅在 Windows 上对其做断言。在演示的深色夹具（#131722）与规范的深色表面（#1f1f1f）对齐之前，dark 用例无法做到精确一致；marker 与 trading 用例（分别有 1,653 与 5,227 个像素不同，最大通道差值 247）受取决于宿主字体的 Windows 限值约束，因此 Linux 不运行它们。CI 的浏览器作业仍在 Windows 上运行；Linux 矩阵是本地证据。

### gpui-fast 证据线

`gpui-fast` CI 作业是证据，而不是门禁：它在 Linux、macOS 与 Windows 上针对 gpui-fast（`longbridge/gpui-fast`，一个保留每个它未察觉变化的视图的 GPUI）构建并测试 GPUI 后端，而库仍依赖 `gpui-pre =0.3.7`，因此宿主不受影响。修订固定在一个文件 `.cargo/gpui-fast.toml` 中；它是 Cargo 从不自行加载的 Cargo 配置片段，用 gpui-fast 的即插即用兼容 crate 修补 `gpui-pre` 与 `gpui-pre-platform`（依赖图中不再保留其他 `gpui-pre` crate）。升级时替换该文件中唯一的修订哈希，它出现在两条修补行上；两者不一致时作业会报错并指明该文件，因为两个修订会解析出两份 GPUI。gpui-fast 固定了一些版本与已提交锁文件不同的 crate（`unicode-properties =0.1.3`，锁文件为 0.1.4），因此作业先在修补下更新锁文件，再不带 `--locked` 构建。本地命令：

```text
cargo update --config .cargo/gpui-fast.toml -p gpui-pre -p gpui-pre-platform
cargo clippy --config .cargo/gpui-fast.toml -p aeris_charts_render_gpui --features gpui-backend --all-targets -- -D warnings   # Linux
cargo test --config .cargo/gpui-fast.toml -p aeris_charts_render_gpui --features gpui-backend --all-targets
cargo run --config .cargo/gpui-fast.toml -p aeris_charts_render_gpui --features gpui-backend --example pixel_parity            # Windows，以及 xvfb + lavapipe 下的 Linux
```

CI 运行同样的命令但不带 `--config`：它把该片段追加到 Cargo 的 home 配置中，因此作业中的每次 cargo 调用都能看到修补，包括缓存的保存步骤——否则其 `cargo metadata` 会剪除 gpui-fast 的 git 检出与构建产物。缓存键包含固定文件的哈希，因此升级会开始新的缓存，并且证据失败时缓存也会保存。Windows 与 Linux 测试框架运行使用 `native-gpui` 与 `gpui-pixel-parity` 作业的门禁。作业上的 `continue-on-error` 使 gpui-fast 的回归、新的第三方发布或 runner 偶发失败不会让 `ci.yml` 变红，因为标签发布工作流要求整个 `ci.yml` 运行成功；失败的步骤仍会把该作业标记为失败，并且每个步骤都有自己的超时，因此挂起只会让该步骤失败，而不会拖到作业超时。就标签发布工作流而言，移除该标志就是全部的升级步骤，但该证据线目前还不可复现：它的更新每次运行都会把 gpui-fast 的传递依赖解析到最新的兼容发布版本，因此可复现的门禁还需要一份已提交的证据线锁文件，用它代替更新覆盖 `Cargo.lock`，再以 `--locked` 构建。在本地运行上述命令之后要恢复已提交的锁文件（`git checkout -- Cargo.lock`）；`GPUI_VIEW_RETENTION=0` 会关闭 gpui-fast 的保留机制，用以区分缺失的 `refresh` 与保留机制的缺陷。

## 发布与诊断的边界

标签发布需要 Rust、包以及可移植的 Chromium/Firefox/WebKit 作业。公共声明与发布策略守卫、V1 夹具、Node 导入以及 pack 冒烟测试是可移植的阻塞检查。确定性的浏览器/后端像素对比以及原生/浏览器夹具是阻塞的可移植 Chromium 检查。已配置的 `perf_gate` 预算严格执行。按机器校准的截图、GPU 计时、堆采样与墙钟时间证据保留在独立的非阻塞诊断步骤中（`gpui-pixel-parity` 作业即其中之一）；已批准的哈希绝不会仅为迁就另一台宿主而更改。
