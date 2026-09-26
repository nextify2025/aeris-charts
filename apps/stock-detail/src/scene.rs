//! Scripted page states for screenshots and demos: `stock-detail --scene <name>`.
//!
//! Scenes pause the live feed so a capture is reproducible, then drive the page through the same
//! public calls the toolbar buttons make.

use aeris_charts_engine::DrawingKind;
use gpui_kit::*;

use crate::market::Period;
use crate::page::StockPage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene {
    /// Daily candles with MA, VOL and MACD, and the crosshair on a recent bar.
    Day,
    /// Today's session: the price line, its average-price line, VOL and MACD.
    Intraday,
    /// Daily candles with BOLL and KDJ and one of each KLineChart drawing tool.
    Drawings,
    /// The light theme with red-up colors: weekly candles with EMA and SAR, VOL and RSI.
    Light,
}

impl Scene {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "day" => Scene::Day,
            "intraday" => Scene::Intraday,
            "drawings" => Scene::Drawings,
            "light" => Scene::Light,
            _ => return None,
        })
    }

    pub fn apply(self, page: &Entity<StockPage>, window: &mut Window, cx: &mut App) {
        page.update(cx, |page, cx| {
            page.set_live(false);
            let chart = page.chart().clone();
            match self {
                Scene::Day => {
                    let count = chart.read(cx).bar_count();
                    let index = count.saturating_sub(24);
                    let price = chart.read(cx).candle(index).map_or(0.0, |bar| bar.close);
                    chart.update(cx, |chart, _| chart.place_crosshair(index, price));
                }
                Scene::Intraday => {
                    page.select_period(Period::Intraday, cx);
                    let index = 196;
                    let price = chart.read(cx).candle(index).map_or(0.0, |bar| bar.close);
                    chart.update(cx, |chart, _| chart.place_crosshair(index, price));
                }
                Scene::Drawings => {
                    chart.update(cx, |chart, _| {
                        chart.toggle_main("MA");
                        chart.toggle_main("BOLL");
                        chart.toggle_sub("MACD");
                        chart.toggle_sub("KDJ");
                        place_drawings(chart);
                        chart.set_tool(Some(DrawingKind::FibonacciLine));
                    });
                }
                Scene::Light => {
                    page.set_dark(false, window, cx);
                    chart.update(cx, |chart, _| chart.set_red_up(true));
                    page.select_period(Period::Week, cx);
                    chart.update(cx, |chart, _| {
                        chart.toggle_main("MA");
                        chart.toggle_main("EMA");
                        chart.toggle_main("SAR");
                        chart.toggle_sub("MACD");
                        chart.toggle_sub("RSI");
                    });
                    let count = chart.read(cx).bar_count();
                    let index = count.saturating_sub(9);
                    let price = chart.read(cx).candle(index).map_or(0.0, |bar| bar.high);
                    chart.update(cx, |chart, _| chart.place_crosshair(index, price));
                }
            }
            cx.notify();
        });
    }
}

/// One of each KLineChart tool, anchored on the visible swings of the daily bars.
fn place_drawings(chart: &mut crate::chart::ChartView) {
    let count = chart.bar_count();
    if count < 150 {
        return;
    }
    let bars = (0..count)
        .filter_map(|index| chart.candle(index))
        .collect::<Vec<_>>();
    let bar = |index: usize| bars[index];
    // The swing low and high of the last ~130 bars anchor the retracement.
    let window = count - 130..count - 4;
    let low = window
        .clone()
        .min_by(|&a, &b| bar(a).low.total_cmp(&bar(b).low))
        .expect("non-empty window");
    let high = window
        .clone()
        .max_by(|&a, &b| bar(a).high.total_cmp(&bar(b).high))
        .expect("non-empty window");
    let (first, second) = if low < high { (low, high) } else { (high, low) };
    let price = |index: usize, high: bool| {
        let bar = bar(index);
        if high {
            bar.high
        } else {
            bar.low
        }
    };
    let fib_from = (first as f64, price(first, first == high));
    let fib_to = (second as f64, price(second, second == high));
    chart.add_drawing(DrawingKind::FibonacciLine, &[fib_from, fib_to], None);

    // A trend line through two lows before the high, extended both ways.
    let a = count - 120;
    let b = count - 70;
    chart.add_drawing(
        DrawingKind::StraightLine,
        &[(a as f64, bar(a).low), (b as f64, bar(b).low)],
        None,
    );
    // A price channel over the latest leg.
    let c = count - 40;
    let d = count - 12;
    chart.add_drawing(
        DrawingKind::PriceChannel,
        &[
            (c as f64, bar(c).low),
            (d as f64, bar(d).low),
            (c as f64 + 10.0, bar(c + 10).high),
        ],
        None,
    );
    chart.add_drawing(
        DrawingKind::SimpleAnnotation,
        &[(high as f64, bar(high).high)],
        Some("阶段高点"),
    );
    let last = bar(count - 1);
    chart.add_drawing(
        DrawingKind::SimpleTag,
        &[((count - 30) as f64, last.close * 1.06)],
        Some("目标价"),
    );
    chart.add_drawing(
        DrawingKind::PriceLine,
        &[((count - 8) as f64, bar(count - 8).close)],
        None,
    );
    chart.add_drawing(
        DrawingKind::VerticalSegment,
        &[
            ((count - 55) as f64, bar(count - 55).low * 0.985),
            ((count - 55) as f64, bar(count - 55).high * 1.015),
        ],
        None,
    );
}
