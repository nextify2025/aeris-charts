//! Longbridge OpenAPI as a [`MarketData`] source (the `longbridge` cargo feature).
//!
//! Credentials come from the environment, as the Longbridge SDK documents:
//! `LONGBRIDGE_APP_KEY`, `LONGBRIDGE_APP_SECRET` and `LONGBRIDGE_ACCESS_TOKEN`.
//!
//! The SDK's blocking context runs every request on its own runtime thread, so nothing here
//! touches GPUI's executor: history loads when a period opens, and the quotes and candlesticks the
//! server pushes queue up until [`MarketData::poll`] drains them on the UI thread.
//!
//! Timestamps become exchange wall-clock seconds (the convention in `market.rs`) in the exchange's
//! time zone, chosen from the symbol suffix (`.US`, `.HK`, `.SH`, `.SZ`, `.SG`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::error::Error;
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use longbridge::blocking::QuoteContextSync;
use longbridge::quote::{
    AdjustType, Candlestick, IntradayLine, Period as ApiPeriod, PushEvent, PushEventDetail,
    SubFlags, TradeSessions,
};
use longbridge::{Config, Decimal};
use rust_decimal::prelude::ToPrimitive;
use time::OffsetDateTime;
use time_tz::{OffsetDateTimeExt, Tz};

use crate::market::{Candle, MarketData, Period, Quote, DAY};

/// Bars requested per period, the API's maximum for one request.
const HISTORY: usize = 1_000;
/// A quote older than this reads as a closed market.
const STALE_SECONDS: i64 = 120;

pub struct LongbridgeMarket {
    ctx: QuoteContextSync,
    symbol: String,
    tz: &'static Tz,
    quote: Quote,
    /// Unix seconds of the latest pushed quote.
    quote_utc: i64,
    /// Bars per period loaded so far, kept current by pushes. `candles` takes `&self`.
    bars: RefCell<HashMap<Period, Vec<Candle>>>,
    pushes: Receiver<PushEvent>,
}

impl LongbridgeMarket {
    /// Connect with credentials from the environment and load `symbol` (for example `AAPL.US`).
    pub fn connect(symbol: &str) -> Result<Self, Box<dyn Error>> {
        let config = Arc::new(Config::from_apikey_env()?);
        let (sender, pushes) = mpsc::channel();
        let ctx = QuoteContextSync::new(config, move |event| {
            // The page has closed when the receiver is gone; nothing is left to notify.
            let _ = sender.send(event);
        });
        let symbol = symbol.to_owned();
        let tz = exchange_time_zone(&symbol);

        let info = ctx
            .static_info([symbol.clone()])?
            .into_iter()
            .next()
            .ok_or_else(|| format!("Longbridge knows no security {symbol}"))?;
        let quote = ctx
            .quote([symbol.clone()])?
            .into_iter()
            .next()
            .ok_or_else(|| format!("Longbridge returned no quote for {symbol}"))?;
        ctx.subscribe([symbol.clone()], SubFlags::QUOTE)?;

        let mut market = Self {
            quote: Quote {
                time: 0,
                trading: false,
                symbol: symbol.clone(),
                name: if info.name_cn.is_empty() {
                    info.name_en.clone()
                } else {
                    info.name_cn.clone()
                },
                exchange: info.exchange.clone(),
                currency: info.currency.clone(),
                last: number(quote.last_done),
                prev_close: number(quote.prev_close),
                open: number(quote.open),
                high: number(quote.high),
                low: number(quote.low),
                volume: quote.volume as f64,
                turnover: number(quote.turnover),
                week52_high: f64::NAN,
                week52_low: f64::NAN,
                shares_outstanding: info.total_shares as f64,
                eps_ttm: number(info.eps_ttm),
                // The SDK names it a yield, but documents it as the dividend per share.
                dividend_per_share: number(info.dividend_yield),
            },
            quote_utc: quote.timestamp.unix_timestamp(),
            ctx,
            symbol,
            tz,
            bars: RefCell::new(HashMap::new()),
            pushes,
        };
        market.quote.time = market.wall_clock(quote.timestamp);
        market.quote.trading = market.fresh();
        let days = market.candles(Period::Day);
        let year = &days[days.len().saturating_sub(252)..];
        market.quote.week52_high = year.iter().map(|bar| bar.high).fold(f64::NAN, f64::max);
        market.quote.week52_low = year.iter().map(|bar| bar.low).fold(f64::NAN, f64::min);
        Ok(market)
    }

    fn wall_clock(&self, time: OffsetDateTime) -> i64 {
        wall_clock(self.tz, time)
    }

    fn candle(&self, stick: &Candlestick, period: Period) -> Candle {
        Candle {
            time: bar_time(self.tz, stick.timestamp, period),
            open: number(stick.open),
            high: number(stick.high),
            low: number(stick.low),
            close: number(stick.close),
            volume: stick.volume as f64,
            turnover: number(stick.turnover),
        }
    }

    fn intraday_candle(&self, line: &IntradayLine) -> Candle {
        let price = number(line.price);
        Candle {
            time: self.wall_clock(line.timestamp),
            open: price,
            high: price,
            low: price,
            close: price,
            volume: line.volume as f64,
            turnover: number(line.turnover),
        }
    }

    fn fresh(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs() as i64);
        now - self.quote_utc < STALE_SECONDS
    }

    fn load(&self, period: Period) -> Result<Vec<Candle>, Box<dyn Error>> {
        let symbol = self.symbol.clone();
        if period == Period::Intraday {
            // Minute candlesticks keep today's line moving.
            self.ctx.subscribe_candlesticks(
                symbol.clone(),
                ApiPeriod::OneMinute,
                TradeSessions::Intraday,
            )?;
            let lines = self.ctx.intraday(symbol, TradeSessions::Intraday)?;
            return Ok(lines
                .iter()
                .map(|line| self.intraday_candle(line))
                .collect());
        }
        let api_period = api_period(period);
        self.ctx
            .subscribe_candlesticks(symbol.clone(), api_period, TradeSessions::Intraday)?;
        let sticks = self.ctx.candlesticks(
            symbol,
            api_period,
            HISTORY,
            AdjustType::ForwardAdjust,
            TradeSessions::Intraday,
        )?;
        Ok(sticks
            .iter()
            .map(|stick| self.candle(stick, period))
            .collect())
    }
}

impl MarketData for LongbridgeMarket {
    fn quote(&self) -> Quote {
        self.quote.clone()
    }

    fn candles(&self, period: Period) -> Vec<Candle> {
        match self.load(period) {
            Ok(bars) => {
                self.bars.borrow_mut().insert(period, bars.clone());
                bars
            }
            Err(error) => {
                eprintln!("stock-detail: loading {} failed: {error}", period.label());
                Vec::new()
            }
        }
    }

    fn latest(&self, period: Period) -> Option<Candle> {
        self.bars.borrow().get(&period)?.last().copied()
    }

    fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.pushes.try_recv() {
            if event.symbol != self.symbol {
                continue;
            }
            match event.detail {
                PushEventDetail::Quote(push) => {
                    self.quote_utc = push.timestamp.unix_timestamp();
                    self.quote.time = self.wall_clock(push.timestamp);
                    self.quote.last = number(push.last_done);
                    self.quote.open = number(push.open);
                    self.quote.high = number(push.high);
                    self.quote.low = number(push.low);
                    self.quote.volume = push.volume as f64;
                    self.quote.turnover = number(push.turnover);
                    changed = true;
                }
                PushEventDetail::Candlestick(push) => {
                    let Some(period) = app_period(push.period) else {
                        continue;
                    };
                    let bar = self.candle(&push.candlestick, period);
                    let mut bars = self.bars.borrow_mut();
                    // Minute candlesticks also advance today's intraday line.
                    let targets: &[Period] = if period == Period::Min1 {
                        &[Period::Min1, Period::Intraday]
                    } else {
                        std::slice::from_ref(&period)
                    };
                    for target in targets {
                        if let Some(series) = bars.get_mut(target) {
                            upsert(series, bar);
                        }
                    }
                    changed = true;
                }
                _ => {}
            }
        }
        let trading = self.fresh();
        changed |= trading != self.quote.trading;
        self.quote.trading = trading;
        changed
    }
}

/// `time` as wall-clock seconds in `tz`: 09:30 local becomes 09:30 UTC.
fn wall_clock(tz: &Tz, time: OffsetDateTime) -> i64 {
    let local = time.to_timezone(tz);
    local.unix_timestamp() + i64::from(local.offset().whole_seconds())
}

/// A bar's time: minute bars keep their wall-clock minute; daily and longer bars keep their date
/// whether the server stamps exchange midnight or UTC midnight.
fn bar_time(tz: &Tz, time: OffsetDateTime, period: Period) -> i64 {
    match period {
        Period::Day | Period::Week | Period::Month => {
            let utc = time.unix_timestamp();
            if utc.rem_euclid(DAY) == 0 {
                utc
            } else {
                let local = wall_clock(tz, time);
                local - local.rem_euclid(DAY)
            }
        }
        _ => wall_clock(tz, time),
    }
}

/// Replace the bar with the same time, or append a newer one.
fn upsert(bars: &mut Vec<Candle>, bar: Candle) {
    match bars.last_mut() {
        Some(last) if last.time == bar.time => *last = bar,
        Some(last) if last.time > bar.time => {}
        _ => bars.push(bar),
    }
}

fn number(value: Decimal) -> f64 {
    value.to_f64().unwrap_or(f64::NAN)
}

fn api_period(period: Period) -> ApiPeriod {
    match period {
        Period::Intraday | Period::Min1 => ApiPeriod::OneMinute,
        Period::Min5 => ApiPeriod::FiveMinute,
        Period::Min15 => ApiPeriod::FifteenMinute,
        Period::Hour1 => ApiPeriod::SixtyMinute,
        Period::Day => ApiPeriod::Day,
        Period::Week => ApiPeriod::Week,
        Period::Month => ApiPeriod::Month,
    }
}

fn app_period(period: ApiPeriod) -> Option<Period> {
    Some(match period {
        ApiPeriod::OneMinute => Period::Min1,
        ApiPeriod::FiveMinute => Period::Min5,
        ApiPeriod::FifteenMinute => Period::Min15,
        ApiPeriod::SixtyMinute => Period::Hour1,
        ApiPeriod::Day => Period::Day,
        ApiPeriod::Week => Period::Week,
        ApiPeriod::Month => Period::Month,
        _ => return None,
    })
}

fn exchange_time_zone(symbol: &str) -> &'static Tz {
    let name = match symbol.rsplit('.').next() {
        Some("US") => "America/New_York",
        Some("HK") => "Asia/Hong_Kong",
        Some("SH" | "SZ") => "Asia/Shanghai",
        Some("SG") => "Asia/Singapore",
        _ => "Etc/UTC",
    };
    time_tz::timezones::get_by_name(name)
        .or_else(|| time_tz::timezones::get_by_name("Etc/UTC"))
        .expect("the time zone database includes Etc/UTC")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unix seconds of a UTC wall-clock time.
    fn utc(date: (i32, u8, u8), hour: u8, minute: u8) -> OffsetDateTime {
        let month = time::Month::try_from(date.1).unwrap();
        time::Date::from_calendar_date(date.0, month, date.2)
            .unwrap()
            .with_hms(hour, minute, 0)
            .unwrap()
            .assume_utc()
    }

    #[test]
    fn minute_bars_read_in_exchange_time_across_daylight_saving() {
        let new_york = exchange_time_zone("AAPL.US");
        // 09:30 New York is 13:30 UTC in summer (EDT) and 14:30 UTC in winter (EST).
        let summer = utc((2026, 7, 1), 13, 30);
        let winter = utc((2026, 1, 5), 14, 30);
        assert_eq!(
            wall_clock(new_york, summer),
            utc((2026, 7, 1), 9, 30).unix_timestamp()
        );
        assert_eq!(
            wall_clock(new_york, winter),
            utc((2026, 1, 5), 9, 30).unix_timestamp()
        );
        let hong_kong = exchange_time_zone("700.HK");
        assert_eq!(
            wall_clock(hong_kong, utc((2026, 9, 25), 1, 30)),
            utc((2026, 9, 25), 9, 30).unix_timestamp()
        );
        assert!(std::ptr::eq(
            exchange_time_zone("600519.SH"),
            exchange_time_zone("000001.SZ")
        ));
        // An unknown market reads in UTC.
        let other = exchange_time_zone("BTCUSD");
        assert_eq!(wall_clock(other, summer), summer.unix_timestamp());
    }

    #[test]
    fn daily_bars_keep_their_date_under_either_stamp() {
        let new_york = exchange_time_zone("AAPL.US");
        let date = utc((2026, 9, 25), 0, 0).unix_timestamp();
        // Stamped at UTC midnight, or at New York midnight (04:00 UTC in September).
        for stamp in [utc((2026, 9, 25), 0, 0), utc((2026, 9, 25), 4, 0)] {
            assert_eq!(bar_time(new_york, stamp, Period::Day), date);
        }
        let hong_kong = exchange_time_zone("700.HK");
        // Hong Kong midnight is 16:00 UTC the day before.
        assert_eq!(
            bar_time(hong_kong, utc((2026, 9, 24), 16, 0), Period::Day),
            date
        );
    }

    #[test]
    fn pushed_bars_replace_the_live_bar_or_append() {
        let bar = |time: i64, close: f64| Candle {
            time,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1.0,
            turnover: close,
        };
        let mut bars = vec![bar(60, 1.0)];
        upsert(&mut bars, bar(60, 2.0));
        upsert(&mut bars, bar(120, 3.0));
        upsert(&mut bars, bar(0, 9.0));
        assert_eq!(
            bars.iter().map(|b| (b.time, b.close)).collect::<Vec<_>>(),
            [(60, 2.0), (120, 3.0)]
        );
    }
}
