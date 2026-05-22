//! US equity market session clock.
//!
//! Anything that depends on "is the market open" goes through here. Hours are
//! evaluated in `America/New_York` (the exchange's wall clock), so the
//! daylight-saving shift is handled by `chrono-tz` rather than by us.
//!
//! Holidays are deliberately NOT modelled: a full exchange-holiday calendar
//! would need yearly upkeep, and getting it wrong costs almost nothing here.
//! On a holiday the demand-driven intraday job just polls a flat market (and
//! only if someone is watching), and the daily-close job fetches one unchanged
//! quote per symbol. Neither risks a rate limit or stores bad data.

use chrono::{DateTime, Datelike, NaiveTime, Utc, Weekday};
use chrono_tz::America::New_York;

/// A point in the US equity trading day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Session {
    /// Outside all trading hours: overnight or weekend.
    Closed,
    /// Pre-market, 04:00–09:30 ET.
    Pre,
    /// Regular session, 09:30–16:00 ET.
    Regular,
    /// After-hours, 16:00–20:00 ET.
    Post,
}

impl Session {
    /// Whether any trading session (pre, regular, or post) is in progress.
    pub fn is_open(self) -> bool {
        !matches!(self, Session::Closed)
    }

    /// A stable lowercase token for the SSE `market` event and the status pill.
    pub fn as_str(self) -> &'static str {
        match self {
            Session::Closed => "closed",
            Session::Pre => "pre",
            Session::Regular => "regular",
            Session::Post => "post",
        }
    }
}

fn at(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).expect("valid wall-clock time")
}

/// The trading session in effect at `now`.
pub fn session_at(now: DateTime<Utc>) -> Session {
    let et = now.with_timezone(&New_York);
    if matches!(et.weekday(), Weekday::Sat | Weekday::Sun) {
        return Session::Closed;
    }
    let t = et.time();
    if t >= at(9, 30) && t < at(16, 0) {
        Session::Regular
    } else if t >= at(4, 0) && t < at(9, 30) {
        Session::Pre
    } else if t >= at(16, 0) && t < at(20, 0) {
        Session::Post
    } else {
        Session::Closed
    }
}

/// The `America/New_York` calendar date (`YYYY-MM-DD`) at `now`. This is the
/// trading day the daily-close job keys its once-per-day guard on.
pub fn et_date(now: DateTime<Utc>) -> String {
    now.with_timezone(&New_York).format("%Y-%m-%d").to_string()
}

/// Whether `now` falls on a weekday in ET (no holiday calendar; see the
/// module note).
pub fn is_et_weekday(now: DateTime<Utc>) -> bool {
    !matches!(
        now.with_timezone(&New_York).weekday(),
        Weekday::Sat | Weekday::Sun
    )
}

/// Whether the regular session has closed for the current ET day: time is at
/// or past 16:05 ET. The five-minute cushion lets the closing print settle
/// before the daily-close job snapshots it.
pub fn after_close(now: DateTime<Utc>) -> bool {
    now.with_timezone(&New_York).time() >= at(16, 5)
}
