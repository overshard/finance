//! `GET /search` — search and browse the symbol universe.
//!
//! With no query the page lists the whole universe (filterable by kind), so it
//! doubles as a browser. With a query it matches against both ticker and
//! company name. When the query names a plausible ticker the universe does not
//! hold yet, the page offers to add it; the Search page's script does that
//! through `POST /api/symbols` (see `routes::symbols`).

use axum::{
    extract::{Query, State},
    response::Response,
    routing::get,
    Router,
};
use serde::Deserialize;

use crate::models::{to_card, Card, SymbolCardRow};
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
    let rows: Vec<SymbolCardRow> = sqlx::query_as(
        "SELECT s.ticker, s.name, s.kind, \
           COALESCE(s.last_price, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1)), \
           COALESCE(s.prev_close, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1 OFFSET 1)) \
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

    let results: Vec<Card> = rows.into_iter().map(to_card).collect();
    let result_count = results.len() as i64;

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
        show_add => show_add,
        add_ticker => query,
    };
    render(&state, "pages/search.html", "/search", extra)
}
