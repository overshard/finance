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
        (StatusCode::INTERNAL_SERVER_ERROR, "template error").into_response()
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
        (StatusCode::INTERNAL_SERVER_ERROR, "render error").into_response()
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
