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

/// The share of a full trading day's volume that should have accumulated by
/// `now`, for proration. During the regular session it is the fraction of the
/// 09:30–16:00 ET session elapsed (floored at 0.02 so the first minutes do not
/// divide by ~0); at every other time it is 1.0, because Yahoo's
/// `regularMarketVolume` then reflects a *complete* session (the prior day's in
/// pre-market, today's after the close). Dividing today's cumulative volume by
/// `avg_full_day * this_fraction` compares it to the volume typically seen by
/// this point in the day, instead of reading "light" all morning.
pub fn volume_session_fraction(now: DateTime<Utc>) -> f64 {
    match session_at(now) {
        Session::Regular => {
            let t = now.with_timezone(&New_York).time();
            let elapsed = (t - at(9, 30)).num_seconds() as f64;
            let total = (at(16, 0) - at(9, 30)).num_seconds() as f64;
            (elapsed / total).clamp(0.02, 1.0)
        }
        _ => 1.0,
    }
}

/// The `America/New_York` calendar date (`YYYY-MM-DD`) at `now`.
// Retained past the Phase-A removal of the daily-close job: the Phase-C
// dashboard resolves "today" / the most-recent trading day for the day graph.
#[allow(dead_code)]
pub fn et_date(now: DateTime<Utc>) -> String {
    now.with_timezone(&New_York).format("%Y-%m-%d").to_string()
}

/// Whether `now` falls on a weekday in ET (no holiday calendar; see the
/// module note).
#[allow(dead_code)] // see et_date: Phase-C market-hours logic.
pub fn is_et_weekday(now: DateTime<Utc>) -> bool {
    !matches!(
        now.with_timezone(&New_York).weekday(),
        Weekday::Sat | Weekday::Sun
    )
}

/// Whether the regular session has closed for the current ET day: time is at
/// or past 16:05 ET.
#[allow(dead_code)] // see et_date: Phase-C market-hours logic.
pub fn after_close(now: DateTime<Utc>) -> bool {
    now.with_timezone(&New_York).time() >= at(16, 5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    // June 2026 is EDT (UTC-4), so ET = UTC - 4h. 2026-06-24 is a Wednesday.
    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    #[test]
    fn session_at_maps_the_trading_day() {
        assert_eq!(session_at(utc(2026, 6, 24, 12, 0)), Session::Pre); // 08:00 ET
        assert_eq!(session_at(utc(2026, 6, 24, 13, 30)), Session::Regular); // 09:30 ET open
        assert_eq!(session_at(utc(2026, 6, 24, 17, 0)), Session::Regular); // 13:00 ET
        assert_eq!(session_at(utc(2026, 6, 24, 20, 0)), Session::Post); // 16:00 ET close
        assert_eq!(session_at(utc(2026, 6, 24, 1, 0)), Session::Closed); // overnight
        assert_eq!(session_at(utc(2026, 6, 27, 17, 0)), Session::Closed); // Saturday
    }

    #[test]
    fn volume_fraction_prorates_only_during_the_regular_session() {
        // Pre-market: regularMarketVolume is the prior full session → 1.0.
        assert_eq!(volume_session_fraction(utc(2026, 6, 24, 12, 0)), 1.0);
        // Midday (13:00 ET): 3.5h of a 6.5h session elapsed ≈ 0.538.
        let mid = volume_session_fraction(utc(2026, 6, 24, 17, 0));
        assert!((mid - 3.5 / 6.5).abs() < 1e-6, "midday fraction was {mid}");
        // After hours and weekends are a complete session → 1.0.
        assert_eq!(volume_session_fraction(utc(2026, 6, 24, 21, 0)), 1.0);
        assert_eq!(volume_session_fraction(utc(2026, 6, 27, 17, 0)), 1.0);
    }

    #[test]
    fn volume_fraction_floors_at_the_open() {
        // Right at the open the elapsed fraction is floored (not ~0) so the
        // morning ratio does not divide by near-zero and explode.
        let at_open = volume_session_fraction(utc(2026, 6, 24, 13, 30));
        assert!(at_open >= 0.02, "expected a floor, got {at_open}");
    }
}
