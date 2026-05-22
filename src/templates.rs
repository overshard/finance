use minijinja::value::Value;
use minijinja::{path_loader, AutoEscape, Environment, Error, Output, State};
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::path::Path;

/// Jinja2-faithful HTML formatter. Does NOT escape `/`, so vite asset URLs
/// like `/static/base-abc123.js` come through clean instead of `&#x2f;...`.
fn jinja2_html_formatter(out: &mut Output, state: &State, value: &Value) -> Result<(), Error> {
    if value.is_safe() {
        write!(out, "{value}").map_err(Error::from)?;
        return Ok(());
    }
    let auto_escape = match state.auto_escape() {
        AutoEscape::Html => true,
        AutoEscape::None => false,
        _ => return minijinja::escape_formatter(out, state, value),
    };
    if !auto_escape {
        write!(out, "{value}").map_err(Error::from)?;
        return Ok(());
    }
    if let Some(s) = value.as_str() {
        write_jinja2_html(out, s).map_err(Error::from)?;
    } else if value.is_undefined() || value.is_none() {
        // emit nothing
    } else {
        let stringified = value.to_string();
        write_jinja2_html(out, &stringified).map_err(Error::from)?;
    }
    Ok(())
}

fn write_jinja2_html(out: &mut Output, s: &str) -> std::fmt::Result {
    let mut last = 0;
    for (i, b) in s.bytes().enumerate() {
        let escape = match b {
            b'&' => "&amp;",
            b'<' => "&lt;",
            b'>' => "&gt;",
            b'"' => "&#34;",
            b'\'' => "&#39;",
            _ => continue,
        };
        if last < i {
            out.write_str(&s[last..i])?;
        }
        out.write_str(escape)?;
        last = i + 1;
    }
    if last < s.len() {
        out.write_str(&s[last..])?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestCtx {
    pub path: String,
}

fn read_manifest(path: &Path) -> JsonValue {
    let text = std::fs::read_to_string(path).unwrap_or_else(|_| "{}".to_string());
    serde_json::from_str(&text).unwrap_or(JsonValue::Null)
}

fn lookup_asset(manifest: &JsonValue, entry: &str, kind: &str) -> String {
    if let Some(chunk) = manifest.get(entry) {
        if kind == "css" {
            if let Some(css_arr) = chunk.get("css").and_then(|v| v.as_array()) {
                if let Some(first) = css_arr.first().and_then(|v| v.as_str()) {
                    return format!("/static/{first}");
                }
            }
        }
        if let Some(file) = chunk.get("file").and_then(|v| v.as_str()) {
            return format!("/static/{file}");
        }
    }
    format!("/static/{entry}")
}

pub fn build_env(templates_dir: &Path, manifest_path: &Path) -> Environment<'static> {
    let mut env = Environment::new();
    env.set_loader(path_loader(templates_dir));
    env.set_formatter(jinja2_html_formatter);

    // Resolve content-hashed Vite asset names. Re-read the manifest per call
    // in debug builds (Vite watch rewrites it); cache it once in release.
    #[cfg(debug_assertions)]
    {
        let path = manifest_path.to_path_buf();
        env.add_function(
            "vite_asset",
            move |entry: String, kind: Option<String>| -> Result<String, Error> {
                let kind = kind.unwrap_or_else(|| "file".to_string());
                Ok(lookup_asset(&read_manifest(&path), &entry, &kind))
            },
        );
    }
    #[cfg(not(debug_assertions))]
    {
        let manifest = read_manifest(manifest_path);
        env.add_function(
            "vite_asset",
            move |entry: String, kind: Option<String>| -> Result<String, Error> {
                let kind = kind.unwrap_or_else(|| "file".to_string());
                Ok(lookup_asset(&manifest, &entry, &kind))
            },
        );
    }

    env.add_filter("money", money_filter);
    env.add_filter("signed", signed_filter);
    env.add_filter("pct", pct_filter);
    env.add_filter("compact", compact_filter);
    env.add_filter("intcomma", intcomma_filter);
    env.add_filter("ago", ago_filter);
    env.add_filter("urlencode", urlencode_filter);

    env
}

/// Best-effort numeric coercion. Returns None for undefined / null / non-numeric
/// so filters can fall back to a placeholder.
fn as_f64(v: &Value) -> Option<f64> {
    if v.is_none() || v.is_undefined() {
        return None;
    }
    if let Some(i) = v.as_i64() {
        return Some(i as f64);
    }
    v.as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| v.to_string().parse().ok())
}

/// Format a number with thousands separators and `dp` decimal places.
fn fmt_grouped(n: f64, dp: usize) -> String {
    let neg = n.is_sign_negative() && n != 0.0;
    let s = format!("{:.*}", dp, n.abs());
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (s, None),
    };
    let mut grouped = String::new();
    for (i, ch) in int.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            grouped.insert(0, ',');
        }
        grouped.insert(0, ch);
    }
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    out.push_str(&grouped);
    if let Some(f) = frac {
        out.push('.');
        out.push_str(&f);
    }
    out
}

/// Empty-value placeholder shown when a metric is missing — an em dash, an
/// unambiguous "no data" mark (a middle dot read as a stray decimal point).
const DASH: &str = "\u{2014}";

/// `1234.5` -> `$1,234.50`
fn money_filter(value: Value) -> Result<String, Error> {
    match as_f64(&value) {
        Some(n) => Ok(format!("${}", fmt_grouped(n, 2))),
        None => Ok(DASH.to_string()),
    }
}

/// `1.2` -> `+1.20`, `-1.2` -> `-1.20`. For absolute price changes.
fn signed_filter(value: Value) -> Result<String, Error> {
    match as_f64(&value) {
        Some(n) => {
            let sign = if n > 0.0 { "+" } else { "" };
            Ok(format!("{sign}{}", fmt_grouped(n, 2)))
        }
        None => Ok(DASH.to_string()),
    }
}

/// `1.234` -> `+1.23%`. For percentage changes.
fn pct_filter(value: Value) -> Result<String, Error> {
    match as_f64(&value) {
        Some(n) => {
            let sign = if n > 0.0 { "+" } else { "" };
            Ok(format!("{sign}{:.2}%", n))
        }
        None => Ok(DASH.to_string()),
    }
}

/// `1_530_000` -> `1.53M`. For volume and market cap.
fn compact_filter(value: Value) -> Result<String, Error> {
    let Some(n) = as_f64(&value) else {
        return Ok(DASH.to_string());
    };
    let abs = n.abs();
    let (scaled, suffix) = if abs >= 1e12 {
        (n / 1e12, "T")
    } else if abs >= 1e9 {
        (n / 1e9, "B")
    } else if abs >= 1e6 {
        (n / 1e6, "M")
    } else if abs >= 1e3 {
        (n / 1e3, "K")
    } else {
        return Ok(fmt_grouped(n, 0));
    };
    Ok(format!("{scaled:.2}{suffix}"))
}

fn intcomma_filter(value: Value) -> Result<String, Error> {
    match as_f64(&value) {
        Some(n) => Ok(fmt_grouped(n, 0)),
        None => Ok(DASH.to_string()),
    }
}

/// Epoch-ms -> a short relative string like `4m ago`.
fn ago_filter(value: Value) -> Result<String, Error> {
    let Some(ms) = value.as_i64() else {
        return Ok(DASH.to_string());
    };
    let secs = (chrono::Utc::now().timestamp_millis() - ms) / 1000;
    Ok(if secs < 5 {
        "just now".to_string()
    } else if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    })
}

fn urlencode_filter(value: Value) -> Result<String, Error> {
    let s = value
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| value.to_string());
    Ok(urlencoding::encode(&s).into_owned())
}
