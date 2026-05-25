use std::time::Duration;

use crate::Config;

/// One shared reqwest client for all outbound data requests.
///
/// rustls negotiates HTTP/2 over ALPN, and gzip/brotli are decompressed
/// transparently. A reusable client is correct here: unlike a latency
/// probe, these calls are plain data fetches with no per-request handshake
/// measurement to preserve.
pub fn build_client(config: &Config) -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(config.user_agent.clone())
        .timeout(Duration::from_secs(25))
        // Yahoo's v10 `quoteSummary` endpoint is crumb-gated and the crumb
        // round-trip requires session cookies — the GET to fc.yahoo.com sets
        // one, which the subsequent /v1/test/getcrumb call must echo. With
        // cookies enabled here, [`YahooProvider::ensure_crumb`] does the
        // dance and the cookies replay automatically on later requests.
        .cookie_store(true)
        .build()
        .expect("reqwest client builds")
}

/// A client for SEC EDGAR requests.
///
/// SEC's fair-access policy asks every consumer to identify itself, so the
/// configured contact email is appended to the User-Agent on these requests
/// only (the public market endpoints get the plain browser string from
/// `build_client`). A `companyfacts` payload can run to several MB, so the
/// timeout is more generous than the default client's.
pub fn build_sec_client(config: &Config) -> reqwest::Client {
    let user_agent = if config.sec_contact_email.is_empty() {
        config.user_agent.clone()
    } else {
        format!("{} {}", config.user_agent, config.sec_contact_email)
    };
    reqwest::Client::builder()
        .user_agent(user_agent)
        .timeout(Duration::from_secs(40))
        .build()
        .expect("reqwest client builds")
}
