use axum::{http::HeaderValue, middleware as axum_middleware, Router};
use minijinja::Environment;
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::routes;
use crate::{db, middleware, templates};

/// A current desktop Chrome string. Outbound data requests carry this so the
/// public upstreams (Yahoo, SEC) see an ordinary-looking browser.
/// Override with FINANCE_USER_AGENT.
const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
    AppleWebKit/537.36 (KHTML, like Gecko) Chrome/132.0.0.0 Safari/537.36";

#[derive(Clone)]
pub struct AppState {
    pub env: Arc<Environment<'static>>,
    pub pool: SqlitePool,
    pub config: Arc<Config>,
    /// Live-data pub/sub hub: shared by the `/stream` route and the scheduler.
    pub hub: Arc<crate::stream::Hub>,
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Project root: where `templates/` and `dist/` live.
    pub root: PathBuf,
    /// Where `db.sqlite3` lives.
    pub data_dir: PathBuf,
    /// Absolute origin for sitemap / og tags. No trailing slash. May be empty.
    pub base_url: String,
    pub site_title: String,
    /// User-Agent sent on every outbound data request.
    pub user_agent: String,
    /// Appended to the User-Agent on sec.gov requests so SEC can identify us.
    pub sec_contact_email: String,
    /// Which `QuoteProvider` impl to use for live data.
    pub quote_provider: String,
}

impl AppState {
    pub async fn from_env() -> anyhow::Result<Self> {
        let root: PathBuf = std::env::var("FINANCE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        let data_dir = std::env::var("FINANCE_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| root.join("data"));
        std::fs::create_dir_all(&data_dir)?;

        let base_url = std::env::var("BASE_URL").unwrap_or_default();
        let site_title =
            std::env::var("FINANCE_TITLE").unwrap_or_else(|_| "Finance".to_string());
        let user_agent = std::env::var("FINANCE_USER_AGENT")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_USER_AGENT.to_string());
        let sec_contact_email = std::env::var("SEC_CONTACT_EMAIL").unwrap_or_default();
        let quote_provider = std::env::var("FINANCE_QUOTE_PROVIDER")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "yahoo".to_string());

        let pool = db::init(&data_dir).await?;

        let templates_dir = root.join("templates");
        let manifest_path = root.join("dist/.vite/manifest.json");
        let env = Arc::new(templates::build_env(&templates_dir, &manifest_path));

        let config = Arc::new(Config {
            root,
            data_dir,
            base_url,
            site_title,
            user_agent,
            sec_contact_email,
            quote_provider,
        });

        let hub = Arc::new(crate::stream::Hub::new());

        Ok(Self {
            env,
            pool,
            config,
            hub,
        })
    }
}

pub fn router(state: AppState) -> Router {
    let dist_dir = state.config.root.join("dist");

    let static_cache = SetResponseHeaderLayer::if_not_present(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000"),
    );

    Router::new()
        .merge(routes::home::router())
        .merge(routes::industries::router())
        .merge(routes::symbols::router())
        .merge(routes::search::router())
        .merge(routes::stream::router())
        .merge(routes::health::router())
        .merge(routes::seo::router())
        .nest_service(
            "/static",
            tower::ServiceBuilder::new()
                .layer(static_cache)
                .service(ServeDir::new(&dist_dir)),
        )
        .fallback(middleware::not_found)
        .layer(axum_middleware::from_fn(middleware::log_requests))
        .with_state(state)
}
