//! Shared `sqlx` row structs and small cross-route view models.

use serde::Serialize;
use sqlx::FromRow;

use crate::compute;

/// A full row of the `symbols` table.
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct SymbolRow {
    pub ticker: String,
    pub name: String,
    pub kind: String,
    pub exchange: Option<String>,
    pub currency: String,
    pub cik: Option<String>,
    pub sector: Option<String>,
    pub industry: Option<String>,
    pub is_seeded: i64,
    pub is_watched: i64,
    pub history_synced_at: Option<i64>,
    pub history_first_date: Option<String>,
    pub history_last_date: Option<String>,
    pub fundamentals_synced_at: Option<i64>,
    pub filings_synced_at: Option<i64>,
    pub last_price: Option<f64>,
    pub prev_close: Option<f64>,
    pub last_quote_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A symbol's price row as selected for a card grid: ticker, name, kind, the
/// price to show, and the close it is changing against.
pub type SymbolCardRow = (String, String, String, Option<f64>, Option<f64>);

/// A compact symbol tile, rendered by the `ticker_card` macro. Shared by the
/// Markets dashboard and the Search page.
#[derive(Serialize)]
pub struct Card {
    pub ticker: String,
    pub name: String,
    pub kind: String,
    pub price: Option<f64>,
    pub change_abs: Option<f64>,
    pub change_pct: Option<f64>,
}

/// Build a [`Card`] from a selected price row, computing the change off the
/// price and its prior close.
pub fn to_card((ticker, name, kind, last, prev): SymbolCardRow) -> Card {
    let (change_abs, change_pct) = match (last, prev) {
        (Some(l), Some(p)) => {
            let c = compute::change(l, p);
            (Some(c.abs), Some(c.pct))
        }
        _ => (None, None),
    };
    Card {
        ticker,
        name,
        kind,
        price: last,
        change_abs,
        change_pct,
    }
}
