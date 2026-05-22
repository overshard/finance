//! `GET /search` — search and browse the symbol universe.
//!
//! With no query the page lists the whole universe (filterable by kind), so it
//! doubles as a browser. With a query it matches against both ticker and
//! company name. When the query names a plausible ticker the universe does not
//! hold yet, the page offers to add it; the Search page's script does that
//! through `POST /api/symbols` (see `routes::symbols`).

use std::collections::HashMap;

use axum::{
    extract::{Query, State},
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use serde::Deserialize;

use crate::compute;
use crate::models::{self, to_card, Card};
use crate::render::render;
use crate::routes::symbols::valid_ticker;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/search", get(search_page))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
    /// Kind filter: `index` | `future` | `etf` | `stock`, or absent / empty for all.
    kind: Option<String>,
}

/// Escape SQL `LIKE` wildcards in user input so a literal `%` or `_` is
/// matched as itself. Paired with `ESCAPE '\'` in the query.
fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

async fn search_page(Query(sq): Query<SearchQuery>, State(state): State<AppState>) -> Response {
    let raw = sq.q.unwrap_or_default();
    // Tickers are stored uppercase; matching is uppercased throughout. `LIKE`
    // is ASCII-case-insensitive, so company names still match in any case.
    let query = raw.trim().to_uppercase();
    // Normalise the kind filter to one of the three known kinds, else "all".
    let kind = match sq.kind.as_deref().map(str::trim).unwrap_or("") {
        k @ ("index" | "future" | "etf" | "stock") => k,
        _ => "",
    };

    let escaped = escape_like(&query);
    let like = format!("%{escaped}%");
    let prefix = format!("{escaped}%");

    // One query covers both browse (empty `q`) and search: the `? = ''` guards
    // make the ticker/name and kind filters no-ops when their input is empty.
    // Ordering puts an exact ticker hit first, then ticker prefix matches,
    // then indexes before futures before ETFs before stocks, then alphabetical.
    type SearchRow = (String, String, String, Option<f64>, Option<f64>, Option<i64>);
    let rows: Vec<SearchRow> = sqlx::query_as(
        "SELECT s.ticker, s.name, s.kind, \
           COALESCE(s.last_price, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1)), \
           COALESCE(s.prev_close, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1 OFFSET 1)), \
           s.last_quote_at \
         FROM symbols s \
         WHERE (? = '' OR s.ticker LIKE ? ESCAPE '\\' OR s.name LIKE ? ESCAPE '\\') \
           AND (? = '' OR s.kind = ?) \
         ORDER BY (s.ticker = ?) DESC, (s.ticker LIKE ? ESCAPE '\\') DESC, \
                  CASE s.kind WHEN 'index' THEN 0 WHEN 'future' THEN 1 \
                              WHEN 'etf' THEN 2 ELSE 3 END, s.ticker \
         LIMIT 240",
    )
    .bind(&query)
    .bind(&like)
    .bind(&like)
    .bind(kind)
    .bind(kind)
    .bind(&query)
    .bind(&prefix)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    // The freshest quote across the matched symbols backs the page's "prices
    // as of ..." caption (PLAN.md Phase 22).
    let asof: Option<i64> = rows.iter().filter_map(|r| r.5).max();
    let mut results: Vec<Card> = rows
        .into_iter()
        .map(|(t, n, k, last, prev, _)| to_card((t, n, k, last, prev)))
        .collect();

    // A search that pinpoints exactly one symbol jumps straight to its page,
    // rather than rendering a single card the user must then click (Phase 21).
    // Browse mode (an empty query) never redirects.
    if !query.is_empty() && results.len() == 1 {
        let target = format!("/s/{}", urlencoding::encode(&results[0].ticker));
        return Redirect::to(&target).into_response();
    }

    let result_count = results.len() as i64;

    // Attach each stock card's strong / fair / weak verdict badge (Phase 20).
    // ETFs, indexes and futures carry no badge — only stocks have the SEC
    // fundamentals a standing is rolled from.
    attach_standings(&state, &mut results).await;

    // Offer "Add" only on a genuine miss: nothing matched, the query is a
    // plausible ticker (one `POST /api/symbols` would accept), and it is not
    // already a symbol the kind filter happened to hide. Requiring zero
    // results keeps the offer from nagging when a name search did find hits.
    let show_add = results.is_empty()
        && valid_ticker(&query).is_some()
        && !sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM symbols WHERE ticker = ?)")
            .bind(&query)
            .fetch_one(&state.pool)
            .await
            .unwrap_or(false);

    let extra = minijinja::context! {
        title => "Search",
        q => raw.trim(),
        kind => kind,
        results => results,
        result_count => result_count,
        asof => asof,
        show_add => show_add,
        add_ticker => query,
    };
    render(&state, "pages/search.html", "/search", extra)
}

/// Fill in the `strength` badge for the stock cards in `cards`, in one batch
/// query over their stored SEC fundamentals. Non-stock cards are left
/// untouched. The badge reflects fundamental strength only — the home page is
/// where the trajectory half is read — so no daily-close series is needed.
async fn attach_standings(state: &AppState, cards: &mut [Card]) {
    let stock_tickers: Vec<&str> = cards
        .iter()
        .filter(|c| c.kind == "stock")
        .map(|c| c.ticker.as_str())
        .collect();
    if stock_tickers.is_empty() {
        return;
    }

    // The `IN` list is built from tickers already in `symbols`, not raw user
    // input, so the placeholder count is bounded and safe.
    let placeholders = vec!["?"; stock_tickers.len()].join(",");
    let sql = format!(
        "SELECT ticker, metric, period, fiscal_year, fiscal_qtr, value \
         FROM fundamentals WHERE ticker IN ({placeholders})"
    );
    let mut q = sqlx::query_as::<_, (String, String, String, i64, Option<i64>, f64)>(&sql);
    for t in &stock_tickers {
        q = q.bind(*t);
    }
    let fact_rows = q.fetch_all(&state.pool).await.unwrap_or_default();

    let mut facts: HashMap<String, Vec<models::FundFact>> = HashMap::new();
    for (ticker, metric, period, fiscal_year, fiscal_qtr, value) in fact_rows {
        facts.entry(ticker).or_default().push(models::FundFact {
            metric,
            period,
            fiscal_year,
            fiscal_qtr,
            value,
        });
    }

    for card in cards.iter_mut().filter(|c| c.kind == "stock") {
        card.strength = facts.get(&card.ticker).and_then(|f| {
            let inputs = models::latest_annual_inputs(f, card.price)?;
            compute::standing(&compute::compute_ratios(&inputs), &[])
        });
    }
}
