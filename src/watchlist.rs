//! Session-scoped dashboard watchlists (Phase C).
//!
//! The dashboard is a personal, editable watchlist with no accounts: a browser
//! is identified by an opaque `fin_sid` cookie, and its symbols live in the
//! `watchlist` table keyed on that sid (see migration 0015). A brand-new
//! browser (no cookie) is minted a sid and seeded with the [`STARTERS`]; an
//! existing cookie's list is used as-is, even when empty, so a user who removes
//! everything is not re-seeded. Clearing cookies loses the list, by design.

use axum::http::{header, HeaderMap};
use sqlx::SqlitePool;

use crate::db::now_ms;

/// The symbols a brand-new browser's watchlist is seeded with. The S&P 500
/// baseline is *not* here — the dashboard always shows it as the comparison
/// baseline; these are the user's editable rows.
pub const STARTERS: &[&str] = &["VTI", "VXUS", "BND", "IAU", "IBIT"];

/// The cookie name that carries the opaque session id.
pub const COOKIE: &str = "fin_sid";

/// The session cookie's lifetime: one year.
const COOKIE_MAX_AGE_SECS: i64 = 365 * 24 * 3600;

/// A resolved browser session: its sid, and the `Set-Cookie` value to send when
/// the session was just minted (a first visit).
pub struct Session {
    pub sid: String,
    pub set_cookie: Option<String>,
}

/// Parse the `fin_sid` value out of the request's `Cookie` header, if present.
/// Accepts only a short hex string (what we mint), so a hand-crafted cookie
/// can't smuggle anything unexpected into the sid.
pub fn sid_from_headers(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    let prefix = format!("{COOKIE}=");
    for part in raw.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix(&prefix) {
            let v = v.trim();
            if !v.is_empty() && v.len() <= 64 && v.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// The `Set-Cookie` header value that persists `sid` for a year.
pub fn set_cookie_value(sid: &str) -> String {
    format!("{COOKIE}={sid}; Path=/; Max-Age={COOKIE_MAX_AGE_SECS}; HttpOnly; SameSite=Lax")
}

/// Resolve the browser's session. With a cookie present, use that sid as-is
/// (its list is whatever the browser arranged, even if empty). With no cookie,
/// mint a new opaque sid, seed the starter watchlist, and return the cookie to
/// set.
pub async fn resolve(pool: &SqlitePool, headers: &HeaderMap) -> Session {
    if let Some(sid) = sid_from_headers(headers) {
        return Session { sid, set_cookie: None };
    }
    // Mint 16 random bytes as hex via SQLite, so no extra crate is needed; fall
    // back to a timestamp-derived id only if that ever fails.
    let sid: String = sqlx::query_scalar("SELECT lower(hex(randomblob(16)))")
        .fetch_one(pool)
        .await
        .unwrap_or_else(|_| format!("{:032x}", now_ms()));
    seed_starters(pool, &sid).await;
    let set_cookie = Some(set_cookie_value(&sid));
    Session { sid, set_cookie }
}

/// Seed a fresh session with the starter symbols (only those that exist in the
/// universe — they all do, but the guard keeps a missing one from inserting a
/// dangling row).
async fn seed_starters(pool: &SqlitePool, sid: &str) {
    let now = now_ms();
    for (i, t) in STARTERS.iter().enumerate() {
        let _ = sqlx::query(
            "INSERT INTO watchlist (sid, ticker, position, added_at) \
             SELECT ?, ?, ?, ? WHERE EXISTS (SELECT 1 FROM symbols WHERE ticker = ?) \
             ON CONFLICT(sid, ticker) DO NOTHING",
        )
        .bind(sid)
        .bind(t)
        .bind(i as i64)
        .bind(now)
        .bind(t)
        .execute(pool)
        .await;
    }
}

/// The watchlist tickers for `sid`, in display order.
pub async fn list(pool: &SqlitePool, sid: &str) -> Vec<String> {
    sqlx::query_scalar("SELECT ticker FROM watchlist WHERE sid = ? ORDER BY position, added_at")
        .bind(sid)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

/// Append `ticker` to `sid`'s watchlist. Assumes the symbol already exists in
/// the universe (the route ensures it first). Idempotent on (sid, ticker).
pub async fn add_ticker(pool: &SqlitePool, sid: &str, ticker: &str) -> sqlx::Result<()> {
    let now = now_ms();
    sqlx::query(
        "INSERT INTO watchlist (sid, ticker, position, added_at) \
         VALUES (?, ?, COALESCE((SELECT MAX(position) + 1 FROM watchlist WHERE sid = ?), 0), ?) \
         ON CONFLICT(sid, ticker) DO NOTHING",
    )
    .bind(sid)
    .bind(ticker)
    .bind(sid)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Remove `ticker` from `sid`'s watchlist.
pub async fn remove_ticker(pool: &SqlitePool, sid: &str, ticker: &str) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM watchlist WHERE sid = ? AND ticker = ?")
        .bind(sid)
        .bind(ticker)
        .execute(pool)
        .await?;
    Ok(())
}
