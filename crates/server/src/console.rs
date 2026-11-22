//! The web console (console/, Vite + React) served from the binary.
//!
//! `console/dist` is embedded at build time (cargo feature `console`, on by default). When it was not built, the
//! embedded folder is a one-page "console not built, run `just console-build`" placeholder (see build.rs), so a
//! plain `cargo build` never needs node. Without the feature the same page is compiled in as a string.
//!
//! It is the router's fallback: any unknown GET that is not an API/auth path gets `index.html` (SPA routing).

use axum::{
    http::{HeaderValue, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};

#[cfg(feature = "console")]
#[derive(rust_embed::RustEmbed)]
#[folder = "$CONSOLE_DIST"]
struct Assets;

/// Paths owned by the server; they never fall through to the SPA.
const RESERVED: &[&str] = &[
    "/api",
    "/mcp",
    "/oauth",
    "/auth",
    "/device",
    "/logout",
    "/healthz",
    "/.well-known",
    "/static",
];

pub fn is_reserved(path: &str) -> bool {
    RESERVED
        .iter()
        .any(|p| path == *p || path.strip_prefix(p).is_some_and(|r| r.starts_with('/')))
}

/// No inline scripts; styles may be inline (Radix and the toaster set a few). `{img}` takes the
/// object storage origin: file links redirect there, so image previews load from it.
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:{img}; font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'";

static CSP_HEADER: std::sync::OnceLock<HeaderValue> = std::sync::OnceLock::new();

/// Let the console load images from the object storage's public origin (call once at startup).
pub fn allow_images_from(origin: &str) {
    let v = CSP.replace("{img}", &format!(" {origin}"));
    if let Ok(h) = HeaderValue::from_str(&v) {
        let _ = CSP_HEADER.set(h);
    }
}

fn csp() -> HeaderValue {
    CSP_HEADER
        .get_or_init(|| HeaderValue::from_str(&CSP.replace("{img}", "")).unwrap())
        .clone()
}

#[cfg(feature = "console")]
fn asset(path: &str) -> Option<(Vec<u8>, String)> {
    let f = Assets::get(path)?;
    Some((f.data.into_owned(), f.metadata.mimetype().to_string()))
}

#[cfg(not(feature = "console"))]
fn asset(path: &str) -> Option<(Vec<u8>, String)> {
    (path == "index.html").then(|| {
        (
            include_bytes!("../assets/console-missing.html").to_vec(),
            "text/html".to_string(),
        )
    })
}

pub async fn serve(method: Method, uri: Uri) -> Response {
    let path = uri.path();
    if (method != Method::GET && method != Method::HEAD) || is_reserved(path) {
        return not_found();
    }
    let name = path.trim_start_matches('/');
    if !name.is_empty()
        && let Some((bytes, mime)) = asset(name)
    {
        let cache = if name.starts_with("assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        return file(bytes, &mime, cache, name == "index.html");
    }
    // a missing file (has an extension) is a 404, not the app shell
    if name.rsplit('/').next().is_some_and(|l| l.contains('.')) {
        return not_found();
    }
    match asset("index.html") {
        Some((bytes, mime)) => {
            let status = if option_env!("CORNTCHAP_CONSOLE_MISSING").is_some() {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::OK
            };
            let mut r = file(bytes, &mime, "no-cache", true);
            *r.status_mut() = status;
            r
        }
        None => not_found(),
    }
}

fn file(bytes: Vec<u8>, mime: &str, cache: &str, shell: bool) -> Response {
    let mut r = (StatusCode::OK, bytes).into_response();
    let h = r.headers_mut();
    if let Ok(v) = HeaderValue::from_str(mime) {
        h.insert(header::CONTENT_TYPE, v);
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_str(cache).unwrap());
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if shell {
        h.insert(header::CONTENT_SECURITY_POLICY, csp());
        h.insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("same-origin"),
        );
        h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    }
    r
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        axum::Json(serde_json::json!({"error": "not_found", "message": "no such route"})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::is_reserved;

    #[test]
    fn reserved_prefixes() {
        for p in [
            "/api/v1/me",
            "/api",
            "/mcp",
            "/auth/oidc/login",
            "/device",
            "/oauth/token",
            "/logout",
            "/healthz",
            "/.well-known/oauth-authorization-server",
        ] {
            assert!(is_reserved(p), "{p}");
        }
        for p in ["/", "/pots/PAY/list", "/cooks", "/authors", "/apiary"] {
            assert!(!is_reserved(p), "{p}");
        }
    }
}
