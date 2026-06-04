//! `POST /api/watchlist` (add) and `POST /api/watchlist/remove` — edit the
//! current browser session's dashboard watchlist (Phase C).
//!
//! Both resolve the session from the `fin_sid` cookie (minting one on a first
//! visit), so they work even if a script calls them before the dashboard set
//! the cookie. Add ensures the symbol is in the universe first (validating /
//! backfilling a brand-new ticker via `ensure_symbol`), then appends it.

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::routes::symbols::ensure_symbol;
use crate::{watchlist, AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/watchlist", post(add))
        .route("/api/watchlist/remove", post(remove))
}

#[derive(Deserialize)]
struct Body {
    ticker: String,
}

#[derive(Serialize)]
struct Resp {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    ticker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Add a symbol to the session's watchlist, validating / backfilling it into
/// the universe first if it is not tracked yet.
async fn add(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Body>) -> Response {
    let session = watchlist::resolve(&state.pool, &headers).await;
    match ensure_symbol(&state, &body.ticker).await {
        Ok(o) => {
            if let Err(e) = watchlist::add_ticker(&state.pool, &session.sid, &o.ticker).await {
                tracing::warn!("watchlist add {}: {e:#}", o.ticker);
                return reply(
                    &session,
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Resp {
                        ok: false,
                        ticker: None,
                        name: None,
                        error: Some("Could not save to your watchlist.".into()),
                    },
                );
            }
            reply(
                &session,
                StatusCode::OK,
                Resp {
                    ok: true,
                    ticker: Some(o.ticker),
                    name: Some(o.name),
                    error: None,
                },
            )
        }
        Err((status, msg)) => reply(
            &session,
            status,
            Resp {
                ok: false,
                ticker: None,
                name: None,
                error: Some(msg),
            },
        ),
    }
}

/// Remove a symbol from the session's watchlist. Idempotent.
async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Body>,
) -> Response {
    let session = watchlist::resolve(&state.pool, &headers).await;
    let ticker = body.ticker.trim().to_uppercase();
    let _ = watchlist::remove_ticker(&state.pool, &session.sid, &ticker).await;
    reply(
        &session,
        StatusCode::OK,
        Resp {
            ok: true,
            ticker: Some(ticker),
            name: None,
            error: None,
        },
    )
}

/// Build the JSON response, attaching the `Set-Cookie` header when the session
/// was just minted.
fn reply(session: &watchlist::Session, status: StatusCode, body: Resp) -> Response {
    let mut resp = (status, Json(body)).into_response();
    if let Some(c) = &session.set_cookie {
        if let Ok(v) = header::HeaderValue::from_str(c) {
            resp.headers_mut().insert(header::SET_COOKIE, v);
        }
    }
    resp
}
