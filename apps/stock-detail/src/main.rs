//! A stock detail page with a full K-line chart, built with GPUI Kit around the Aeris chart
//! engine: KLineChart's 27 indicators and its drawing tools, period switching, and a live
//! (simulated) feed.
//!
//! ```text
//! cargo run --release                          # the page, with the simulated feed
//! cargo run --release -- --scene drawings      # a scripted state (day, intraday, drawings, light)
//! cargo run --release --features longbridge -- --symbol AAPL.US   # a live Longbridge feed
//! ```

mod chart;
#[cfg(feature = "longbridge")]
mod longbridge;
mod market;
mod page;
mod scene;

use std::borrow::Cow;

use gpui_kit::assets::{icon_assets, Assets};
use gpui_kit::component::{Root, TitleBar};
use gpui_kit::*;

use market::{MarketData, SimulatedMarket};
use page::StockPage;
use scene::Scene;

gpui_kit::actions!(stock_detail, [Quit]);

// Only the icons this page uses are embedded; the component library's defaults come from `Assets`.
icon_assets!(
    PageIcons,
    [
        AlignVerticalSpaceBetween,
        ArrowUpRight,
        BadgeDollarSign,
        ChartCandlestick,
        ChevronDown,
        ChevronRight,
        Equal,
        GitCommitHorizontal,
        GitCommitVertical,
        Magnet,
        MessageSquare,
        Minus,
        Moon,
        MousePointer2,
        MoveDiagonal,
        MoveRight,
        MoveUp,
        Pause,
        PenLine,
        Play,
        Rows3,
        SeparatorVertical,
        Slash,
        Square,
        Sun,
        Tag,
        Trash,
        TrendingDown,
        TrendingUp,
        Type,
        Waypoints,
    ]
);

/// The page's icons first, then the component library's.
struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match PageIcons.load(path)? {
            Some(bytes) => Ok(Some(bytes)),
            None => Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = PageIcons.list(path)?;
        paths.extend(Assets.list(path)?);
        Ok(paths)
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut scene = None;
    let mut symbol = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--scene" => {
                let name = args.next().unwrap_or_default();
                scene = Some(Scene::parse(&name).unwrap_or_else(|| {
                    eprintln!("unknown scene {name:?}; expected day, intraday, drawings or light");
                    std::process::exit(2);
                }));
            }
            "--symbol" => symbol = args.next(),
            other => {
                eprintln!(
                    "unknown argument {other:?}; usage: stock-detail [--scene NAME] [--symbol CODE]"
                );
                std::process::exit(2);
            }
        }
    }
    let market = open_market(symbol);

    gpui_kit::application()
        .with_assets(AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
            cx.set_menus([Menu::new("Aeris 行情").items([MenuItem::action("退出", Quit)])]);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);

            let bounds = Bounds::centered(None, size(px(1440.0), px(900.0)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(960.0), px(640.0))),
                ..TitleBar::window_options()
            };
            cx.spawn(async move |cx| {
                cx.open_window(options, |window, cx| {
                    let page = cx.new(|cx| StockPage::new(market, window, cx));
                    if let Some(scene) = scene {
                        scene.apply(&page, window, cx);
                    }
                    cx.new(|cx| Root::new(page, window, cx))
                })
                .expect("failed to open the window");
            })
            .detach();
        });
}

/// The simulated feed, or Longbridge for `--symbol` when built with the `longbridge` feature.
fn open_market(symbol: Option<String>) -> Box<dyn MarketData> {
    let Some(symbol) = symbol else {
        return Box::new(SimulatedMarket::new());
    };
    #[cfg(feature = "longbridge")]
    {
        match longbridge::LongbridgeMarket::connect(&symbol) {
            Ok(market) => Box::new(market),
            Err(error) => {
                eprintln!("stock-detail: cannot open {symbol} on Longbridge: {error}");
                std::process::exit(1);
            }
        }
    }
    #[cfg(not(feature = "longbridge"))]
    {
        eprintln!("stock-detail: --symbol {symbol} needs a build with `--features longbridge`");
        std::process::exit(2);
    }
}
