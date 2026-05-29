use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use chrono::Datelike;

use crate::templates::RequestCtx;
use crate::AppState;

/// Render a template to a String with the standard page context injected.
///
/// Every template can rely on `request`, `now`, `site`, and `base_url` being
/// present; callers pass page-specific fields through `extra`.
pub fn render_to_string(
    state: &AppState,
    template: &str,
    path: &str,
    extra: minijinja::Value,
) -> Result<String, Response> {
    let tmpl = state.env.get_template(template).map_err(|e| {
        tracing::error!("template '{}': {}", template, e);
        server_error(state, path, &format!("template '{template}': {e}"))
    })?;
    tmpl.render(minijinja::context! {
        request => RequestCtx { path: path.to_string() },
        now => minijinja::context! { year => chrono::Local::now().year() },
        site => minijinja::context! {
            title => &state.config.site_title,
            base_url => &state.config.base_url,
        },
        base_url => &state.config.base_url,
        ..extra
    })
    .map_err(|e| {
        tracing::error!("render '{}': {}", template, e);
        // minijinja's `Display` carries the bare error; the source chain carries
        // the location and line span, which is what the operator needs to look at.
        let mut detail = format!("render '{template}': {e}");
        let mut source = std::error::Error::source(&e);
        while let Some(s) = source {
            detail.push_str(&format!("\n  caused by: {s}"));
            source = s.source();
        }
        server_error(state, path, &detail)
    })
}

/// Convenience wrapper around `render_to_string` for HTML responses.
pub fn render(state: &AppState, template: &str, path: &str, extra: minijinja::Value) -> Response {
    match render_to_string(state, template, path, extra) {
        Ok(body) => Html(body).into_response(),
        Err(resp) => resp,
    }
}

/// The themed 404 page with a 404 status. Used by the router fallback and by
/// routes that look up a missing resource (e.g. an unknown ticker).
pub fn not_found(state: &AppState) -> Response {
    let body = render(
        state,
        "pages/not_found.html",
        "/404",
        minijinja::context! { title => "Not found" },
    );
    (StatusCode::NOT_FOUND, body).into_response()
}

/// The themed 500 page with the underlying error detail. Single-operator app
/// (no public sign-up, see `PLAN.md`), so leaking the message back is fine.
/// It is the whole point of the page. Falls back to plain text if the error
/// page itself fails to render, so we never recurse.
fn server_error(state: &AppState, path: &str, detail: &str) -> Response {
    let ctx = minijinja::context! {
        title => "Page failed to render",
        path => path,
        detail => detail,
    };
    let body = state
        .env
        .get_template("pages/error.html")
        .and_then(|t| {
            t.render(minijinja::context! {
                request => RequestCtx { path: path.to_string() },
                now => minijinja::context! { year => chrono::Local::now().year() },
                site => minijinja::context! {
                    title => &state.config.site_title,
                    base_url => &state.config.base_url,
                },
                base_url => &state.config.base_url,
                ..ctx
            })
        });
    match body {
        Ok(html) => (StatusCode::INTERNAL_SERVER_ERROR, Html(html)).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, detail.to_string()).into_response(),
    }
}
