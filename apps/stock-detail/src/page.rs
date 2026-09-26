//! The stock detail page: quote header, period and indicator bars, the drawing toolbar, and the
//! K-line chart, fed by a [`MarketData`] source.

use std::time::Duration;

use aeris_charts_engine::klinechart::{Indicator, Placement, NAMES};
use aeris_charts_engine::DrawingKind;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{
    h_flex, v_flex, ActiveTheme, Disableable, Icon, Selectable, Sizable, StyledExt, Theme,
    ThemeMode, TitleBar,
};
use gpui_kit::*;

use crate::chart::{
    css_hsla, format_time, format_volume, hsla_css, market_colors, ChartColors, ChartView,
};
use crate::market::{MarketData, Period, Quote};

/// Sub-pane indicators shown as buttons; the rest sit in the "more" menu.
const PINNED_SUBS: [&str; 4] = ["VOL", "MACD", "KDJ", "RSI"];
/// How often the live feed is polled.
const TICK: Duration = Duration::from_millis(1_000);

struct Tool {
    kind: DrawingKind,
    label: &'static str,
    icon: IconName,
}

/// Drawing tools grouped like KLineChart Pro's toolbar; each group remembers its last tool.
struct ToolGroup {
    label: &'static str,
    tools: &'static [Tool],
}

const fn tool(kind: DrawingKind, label: &'static str, icon: IconName) -> Tool {
    Tool { kind, label, icon }
}

const TOOL_GROUP_COUNT: usize = 7;

static TOOL_GROUPS: [ToolGroup; TOOL_GROUP_COUNT] = [
    ToolGroup {
        label: "趋势线",
        tools: &[
            tool(DrawingKind::TrendLine, "线段", IconName::Slash),
            tool(DrawingKind::StraightLine, "直线", IconName::MoveDiagonal),
            tool(DrawingKind::RayLine, "射线", IconName::ArrowUpRight),
            tool(DrawingKind::PriceLine, "价格线", IconName::BadgeDollarSign),
        ],
    },
    ToolGroup {
        label: "水平/垂直线",
        tools: &[
            tool(DrawingKind::HorizontalLine, "水平直线", IconName::Minus),
            tool(DrawingKind::HorizontalRay, "水平射线", IconName::MoveRight),
            tool(
                DrawingKind::HorizontalSegment,
                "水平线段",
                IconName::GitCommitHorizontal,
            ),
            tool(
                DrawingKind::VerticalLine,
                "垂直直线",
                IconName::SeparatorVertical,
            ),
            tool(DrawingKind::VerticalRay, "垂直射线", IconName::MoveUp),
            tool(
                DrawingKind::VerticalSegment,
                "垂直线段",
                IconName::GitCommitVertical,
            ),
        ],
    },
    ToolGroup {
        label: "通道",
        tools: &[
            tool(DrawingKind::ParallelLine, "平行直线", IconName::Equal),
            tool(DrawingKind::PriceChannel, "价格通道线", IconName::Rows3),
        ],
    },
    ToolGroup {
        label: "斐波那契",
        tools: &[tool(
            DrawingKind::FibonacciLine,
            "斐波那契回撤",
            IconName::AlignVerticalSpaceBetween,
        )],
    },
    ToolGroup {
        label: "标注",
        tools: &[
            tool(
                DrawingKind::SimpleAnnotation,
                "注解",
                IconName::MessageSquare,
            ),
            tool(DrawingKind::SimpleTag, "标签", IconName::Tag),
            tool(DrawingKind::Text, "文本", IconName::Type),
        ],
    },
    ToolGroup {
        label: "形状",
        tools: &[
            tool(DrawingKind::Rectangle, "矩形", IconName::Square),
            tool(DrawingKind::Path, "折线", IconName::Waypoints),
            tool(DrawingKind::Brush, "画笔", IconName::PenLine),
        ],
    },
    ToolGroup {
        label: "仓位",
        tools: &[
            tool(DrawingKind::LongPosition, "多头仓位", IconName::TrendingUp),
            tool(
                DrawingKind::ShortPosition,
                "空头仓位",
                IconName::TrendingDown,
            ),
        ],
    },
];

pub struct StockPage {
    market: Box<dyn MarketData>,
    quote: Quote,
    chart: Entity<ChartView>,
    /// The tool each toolbar group arms on a plain click.
    group_tools: [usize; TOOL_GROUP_COUNT],
    live: bool,
    _ticker: Task<()>,
}

impl StockPage {
    pub fn new(market: Box<dyn MarketData>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Theme::change(ThemeMode::Dark, Some(window), cx);
        let chart = cx.new(ChartView::new);
        let ticker = cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(TICK).await;
            if this.update(cx, |page, cx| page.tick(cx)).is_err() {
                break;
            }
        });
        let mut page = Self {
            quote: market.quote(),
            market,
            chart,
            group_tools: [0; TOOL_GROUP_COUNT],
            live: true,
            _ticker: ticker,
        };
        page.sync_chart_colors(cx);
        page.select_period(Period::Day, cx);
        page.chart.update(cx, |chart, _| {
            chart.toggle_main("MA");
            chart.toggle_sub("VOL");
            chart.toggle_sub("MACD");
        });
        page
    }

    pub fn chart(&self) -> &Entity<ChartView> {
        &self.chart
    }

    pub fn set_live(&mut self, live: bool) {
        self.live = live;
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        if !self.live || !self.market.poll() {
            return;
        }
        self.quote = self.market.quote();
        let period = self.chart.read(cx).period();
        if let Some(bar) = self.market.latest(period) {
            self.chart.update(cx, |chart, cx| {
                chart.update_latest(bar);
                cx.notify();
            });
        }
        cx.notify();
    }

    pub fn select_period(&mut self, period: Period, cx: &mut Context<Self>) {
        let candles = self.market.candles(period);
        let prev_close = self.quote.prev_close;
        self.chart.update(cx, |chart, cx| {
            chart.set_candles(period, candles, prev_close);
            cx.notify();
        });
        cx.notify();
    }

    pub fn set_dark(&mut self, dark: bool, window: &mut Window, cx: &mut Context<Self>) {
        Theme::change(
            if dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
            Some(window),
            cx,
        );
        self.sync_chart_colors(cx);
        cx.notify();
    }

    /// The chart takes its chrome colors from the application theme.
    fn sync_chart_colors(&mut self, cx: &mut Context<Self>) {
        let theme = cx.theme();
        let colors = ChartColors {
            background: hsla_css(theme.background),
            text: hsla_css(theme.muted_foreground),
            grid: hsla_css(theme.border.opacity(0.45)),
            border: hsla_css(theme.border),
            crosshair: hsla_css(theme.muted_foreground),
            dark: theme.is_dark(),
        };
        self.chart.update(cx, |chart, cx| {
            chart.set_colors(colors);
            cx.notify();
        });
    }

    fn arm_tool(&mut self, group: usize, index: usize, cx: &mut Context<Self>) {
        self.group_tools[group] = index;
        let kind = TOOL_GROUPS[group].tools[index].kind;
        self.chart.update(cx, |chart, cx| {
            let next = (chart.active_tool() != Some(kind)).then_some(kind);
            chart.set_tool(next);
            cx.notify();
        });
        cx.notify();
    }

    // ---- header -----------------------------------------------------------------------------

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let dark = theme.is_dark();
        let red_up = self.chart.read(cx).red_up();
        h_flex()
            .w_full()
            .justify_between()
            .pr_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(Icon::new(IconName::ChartCandlestick).text_color(theme.primary))
                    .child(div().text_sm().font_semibold().child("Aeris 行情"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("GPUI Kit · Aeris Charts · KLineChart 指标"),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("live")
                            .icon(if self.live {
                                IconName::Pause
                            } else {
                                IconName::Play
                            })
                            .label(if self.live { "实时" } else { "已暂停" })
                            .ghost()
                            .xsmall()
                            .tooltip("暂停或继续模拟行情推送")
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.live = !page.live;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("convention")
                            .label(if red_up {
                                "红涨绿跌"
                            } else {
                                "绿涨红跌"
                            })
                            .ghost()
                            .xsmall()
                            .tooltip("切换涨跌颜色")
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.chart.update(cx, |chart, cx| {
                                    let red_up = chart.red_up();
                                    chart.set_red_up(!red_up);
                                    cx.notify();
                                });
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("theme")
                            .icon(if dark { IconName::Sun } else { IconName::Moon })
                            .ghost()
                            .xsmall()
                            .tooltip(if dark { "浅色主题" } else { "深色主题" })
                            .on_click(cx.listener(move |page, _, window, cx| {
                                page.set_dark(!dark, window, cx);
                            })),
                    ),
            )
    }

    fn render_quote(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let quote = &self.quote;
        let red_up = self.chart.read(cx).red_up();
        let (up, down) = market_colors(red_up);
        let change = quote.change();
        let color = css_hsla(if change >= 0.0 { up } else { down });
        let stat = |label: &'static str, value: String, color: Hsla| {
            v_flex()
                .gap_0p5()
                .min_w(px(92.0))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(label),
                )
                .child(div().text_sm().text_color(color).child(value))
        };
        let direction = |value: f64| css_hsla(if value >= quote.prev_close { up } else { down });
        let fg = theme.foreground;
        let price = |value: f64| format!("{value:.2}");
        let status = format!(
            "{} · {} 美东",
            if quote.trading {
                "交易中"
            } else {
                "已收盘"
            },
            format_time(quote.time, Period::Intraday)
        );
        h_flex()
            .w_full()
            .px_5()
            .py_3()
            .gap_8()
            .items_end()
            .border_b_1()
            .border_color(theme.border)
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_baseline()
                            .child(div().text_xl().font_bold().child(quote.symbol.clone()))
                            .child(div().text_base().child(quote.name.clone()))
                            .child(
                                div()
                                    .px_1p5()
                                    .rounded_sm()
                                    .bg(theme.muted)
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(quote.exchange.clone()),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .items_baseline()
                            .child(
                                div()
                                    .text_3xl()
                                    .font_semibold()
                                    .text_color(color)
                                    .child(price(quote.last)),
                            )
                            .child(
                                div().text_base().text_color(color).child(format!(
                                    "{change:+.2}  {:+.2}%",
                                    quote.change_percent()
                                )),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(format!("{} · {}", quote.currency, status)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_x_6()
                    .gap_y_2()
                    .flex_1()
                    .child(stat("今开", price(quote.open), direction(quote.open)))
                    .child(stat("最高", price(quote.high), direction(quote.high)))
                    .child(stat("最低", price(quote.low), direction(quote.low)))
                    .child(stat("昨收", price(quote.prev_close), fg))
                    .child(stat("成交量", format_volume(quote.volume), fg))
                    .child(stat("成交额", format_volume(quote.turnover), fg))
                    .child(stat("振幅", format!("{:.2}%", quote.amplitude()), fg))
                    .child(stat("总市值", format_volume(quote.market_cap()), fg))
                    .child(stat(
                        "市盈率 TTM",
                        quote
                            .pe_ttm()
                            .map_or_else(|| "亏损".into(), |pe| format!("{pe:.2}")),
                        fg,
                    ))
                    .child(stat(
                        "股息率",
                        format!("{:.2}%", quote.dividend_yield()),
                        fg,
                    ))
                    .child(stat("52周最高", price(quote.week52_high), fg))
                    .child(stat("52周最低", price(quote.week52_low), fg)),
            )
    }

    // ---- chart bars -------------------------------------------------------------------------

    fn render_chart_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let chart = self.chart.read(cx);
        let period = chart.period();
        let intraday = period == Period::Intraday;
        let label = |text: &'static str| {
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .px_1()
                .child(text)
        };
        let divider = || div().w(px(1.0)).h(px(16.0)).mx_2().bg(theme.border);

        let periods = Period::ALL.iter().enumerate().map(|(index, &p)| {
            Button::new(("period", index))
                .label(p.label())
                .ghost()
                .xsmall()
                .selected(p == period)
                .on_click(cx.listener(move |page, _, _, cx| page.select_period(p, cx)))
        });

        let main_names = NAMES
            .iter()
            .copied()
            .filter(|name| *name != "AVP")
            .filter(|name| {
                Indicator::from_name(name).is_some_and(|i| i.placement() == Placement::Price)
            });
        let mains = main_names.enumerate().map(|(index, name)| {
            Button::new(("main", index))
                .label(name)
                .ghost()
                .xsmall()
                .selected(chart.has_main(name))
                .disabled(intraday)
                .on_click(cx.listener(move |page, _, _, cx| {
                    page.chart.update(cx, |chart, cx| {
                        chart.toggle_main(name);
                        cx.notify();
                    });
                    cx.notify();
                }))
        });

        let subs = PINNED_SUBS.iter().enumerate().map(|(index, &name)| {
            Button::new(("sub", index))
                .label(name)
                .ghost()
                .xsmall()
                .selected(chart.has_sub(name))
                .on_click(cx.listener(move |page, _, _, cx| {
                    page.chart.update(cx, |chart, cx| {
                        chart.toggle_sub(name);
                        cx.notify();
                    });
                    cx.notify();
                }))
        });
        let more_active = NAMES
            .iter()
            .filter(|name| !PINNED_SUBS.contains(name))
            .any(|name| chart.has_sub(name));
        let page = cx.entity();
        let more = Button::new("more-subs")
            .label("更多")
            .icon(IconName::ChevronDown)
            .ghost()
            .xsmall()
            .selected(more_active)
            .dropdown_menu(move |mut menu, _, cx| {
                let chart = page.read(cx).chart.clone();
                for &name in NAMES.iter().filter(|name| {
                    !PINNED_SUBS.contains(name)
                        && Indicator::from_name(name)
                            .is_some_and(|i| i.placement() == Placement::Pane)
                }) {
                    let checked = chart.read(cx).has_sub(name);
                    let title =
                        Indicator::from_name(name).map_or_else(|| name.into(), |i| i.title());
                    let chart = chart.clone();
                    menu = menu.item(PopupMenuItem::new(title).checked(checked).on_click(
                        move |_, _, cx| {
                            chart.update(cx, |chart, cx| {
                                chart.toggle_sub(name);
                                cx.notify();
                            });
                        },
                    ));
                }
                menu
            });

        h_flex()
            .w_full()
            .px_3()
            .py_1()
            .gap_0p5()
            .border_b_1()
            .border_color(theme.border)
            .children(periods)
            .child(divider())
            .child(label("主图"))
            .children(mains)
            .child(divider())
            .child(label("副图"))
            .children(subs)
            .child(more)
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let chart = self.chart.read(cx);
        let active = chart.active_tool();
        let magnet = chart.magnet();
        let has_drawings = chart.drawing_count() > 0;
        let page = cx.entity();

        let groups = TOOL_GROUPS.iter().enumerate().map(|(group_index, group)| {
            let current = &group.tools[self.group_tools[group_index]];
            let in_group = group.tools.iter().any(|tool| Some(tool.kind) == active);
            let icon = group
                .tools
                .iter()
                .find(|tool| Some(tool.kind) == active)
                .unwrap_or(current)
                .icon;
            let mut row = h_flex().gap_0().child(
                Button::new(("tool", group_index))
                    .icon(icon)
                    .ghost()
                    .small()
                    .selected(in_group)
                    .tooltip(if group.tools.len() == 1 {
                        group.tools[0].label
                    } else {
                        current.label
                    })
                    .on_click(cx.listener(move |page, _, _, cx| {
                        let index = page.group_tools[group_index];
                        page.arm_tool(group_index, index, cx);
                    })),
            );
            if group.tools.len() > 1 {
                let page = page.clone();
                row = row.child(
                    Button::new(("tool-menu", group_index))
                        .icon(IconName::ChevronRight)
                        .ghost()
                        .xsmall()
                        .compact()
                        .tooltip(group.label)
                        .dropdown_menu(move |mut menu, _, cx| {
                            let active = page.read(cx).chart.read(cx).active_tool();
                            for (index, tool) in group.tools.iter().enumerate() {
                                let page = page.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(tool.label)
                                        .icon(Icon::new(tool.icon))
                                        .checked(active == Some(tool.kind))
                                        .on_click(move |_, _, cx| {
                                            page.update(cx, |page, cx| {
                                                page.arm_tool(group_index, index, cx);
                                            });
                                        }),
                                );
                            }
                            menu
                        }),
                );
            }
            row
        });

        v_flex()
            .h_full()
            .w(px(56.0))
            .py_2()
            .gap_1()
            .items_center()
            .border_r_1()
            .border_color(theme.border)
            .child(
                Button::new("cursor")
                    .icon(IconName::MousePointer2)
                    .ghost()
                    .small()
                    .selected(active.is_none())
                    .tooltip("光标")
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.chart.update(cx, |chart, cx| {
                            chart.set_tool(None);
                            cx.notify();
                        });
                        cx.notify();
                    })),
            )
            .children(groups)
            .child(div().w(px(24.0)).h(px(1.0)).my_1().bg(theme.border))
            .child(
                Button::new("magnet")
                    .icon(IconName::Magnet)
                    .ghost()
                    .small()
                    .selected(magnet)
                    .tooltip("磁吸：锚点吸附到 K 线价格")
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.chart.update(cx, |chart, _| chart.set_magnet(!magnet));
                        cx.notify();
                    })),
            )
            .child(
                Button::new("clear")
                    .icon(IconName::Trash)
                    .ghost()
                    .small()
                    .disabled(!has_drawings)
                    .tooltip("清除全部画线")
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.chart.update(cx, |chart, cx| {
                            chart.clear_drawings();
                            cx.notify();
                        });
                        cx.notify();
                    })),
            )
    }
}

impl Render for StockPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (background, foreground) = (theme.background, theme.foreground);
        v_flex()
            .size_full()
            .bg(background)
            .text_color(foreground)
            .child(TitleBar::new().child(self.render_title_bar(cx)))
            .child(self.render_quote(cx))
            .child(self.render_chart_bar(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(self.render_toolbar(cx))
                    .child(div().flex_1().h_full().min_w_0().child(self.chart.clone())),
            )
    }
}
