//! corntchap app shell.
//!
//! One webview. It starts on the bundled server-picker page (`../src`), then navigates to the
//! console served by the user's own corntchap server, so cookies, OIDC redirects and SSE stay
//! same-origin with the server. The remote origin gets no IPC (see `capabilities/default.json`).

use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder, webview::Color};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_opener::OpenerExt;

const WINDOW: &str = "main";
const CONFIG_FILE: &str = "server.json";
/// How long after the console starts `/auth/oidc/login` any https host may be navigated to.
const LOGIN_WINDOW: Duration = Duration::from_secs(10 * 60);

/// Sets `html[data-native]` on every page (the console's `html[data-native]` CSS keys off it) and gives
/// a way back to the picker without IPC: hold three fingers on the screen for ~1.5s.
const INIT_SCRIPT: &str = r#"
(function () {
  function mark() { if (document.documentElement) document.documentElement.dataset.native = "1"; }
  mark();
  document.addEventListener("readystatechange", mark);
  document.addEventListener("DOMContentLoaded", mark);
  if (window.top !== window) return;
  var timer = null;
  function cancel() { if (timer) { clearTimeout(timer); timer = null; } }
  document.addEventListener("touchstart", function (e) {
    cancel();
    if (e.touches.length === 3) timer = setTimeout(function () { location.href = "corntchap://server"; }, 1500);
  }, { passive: true });
  document.addEventListener("touchend", cancel, { passive: true });
  document.addEventListener("touchcancel", cancel, { passive: true });
})();
"#;

#[derive(Default, Serialize, Deserialize, Clone)]
struct Saved {
    /// Origin of the user's server, e.g. `https://corntchap.camer.digital`.
    server: Option<String>,
    /// Hosts the OIDC login redirected through (learned, then allowed for navigation).
    #[serde(default)]
    idp_hosts: BTreeSet<String>,
}

struct State {
    path: PathBuf,
    saved: Mutex<Saved>,
    login_until: Mutex<Option<Instant>>,
}

impl State {
    fn persist(&self) {
        let saved = self.saved.lock().unwrap().clone();
        if let Ok(json) = serde_json::to_vec_pretty(&saved) {
            if let Some(dir) = self.path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&self.path, json);
        }
    }
}

/// The bundled page's origin (where IPC works).
fn local_base() -> &'static str {
    if cfg!(any(windows, target_os = "android")) {
        "http://tauri.localhost"
    } else {
        "tauri://localhost"
    }
}

fn is_local(url: &Url) -> bool {
    match url.scheme() {
        "tauri" | "about" | "data" | "blob" => true,
        "http" | "https" => matches!(url.host_str(), Some("tauri.localhost")),
        _ => false,
    }
}

fn origin_of(url: &Url) -> Option<String> {
    matches!(url.scheme(), "http" | "https").then(|| url.origin().ascii_serialization())
}

/// Accepts `https://host[:port][/anything]` and returns the bare origin.
fn parse_origin(raw: &str) -> Result<Url, String> {
    let raw = raw.trim();
    let with_scheme = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("https://{raw}")
    };
    let url = Url::parse(&with_scheme).map_err(|_| "That is not a valid address.".to_owned())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("Use an http(s) address.".into());
    }
    Url::parse(&url.origin().ascii_serialization()).map_err(|e| e.to_string())
}

/// `GET /healthz`, falling back to the OAuth metadata document. Native request: no CORS.
fn check_server(origin: &Url) -> Result<(), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .build()
        .into();
    let mut last = String::new();
    for path in ["/healthz", "/.well-known/oauth-authorization-server"] {
        let url = format!("{}{}", origin.as_str().trim_end_matches('/'), path);
        match agent.get(&url).call() {
            Ok(r) if r.status().is_success() => return Ok(()),
            Ok(r) => last = format!("{path} answered {}", r.status()),
            Err(e) => last = e.to_string(),
        }
    }
    Err(format!(
        "Could not reach a corntchap server there ({last})."
    ))
}

fn navigate(app: &AppHandle, url: &str) {
    if let (Some(win), Ok(url)) = (app.get_webview_window(WINDOW), Url::parse(url)) {
        let _ = win.navigate(url);
    }
}

fn show_picker(app: &AppHandle, query: &str) {
    navigate(app, &format!("{}/index.html{query}", local_base()));
}

/// `corntchap://server?url=https://...` opens the picker prefilled; `corntchap://server` just opens it.
fn handle_deep_link(app: &AppHandle, url: &Url) {
    if url.scheme() != "corntchap" || url.host_str() != Some("server") {
        return;
    }
    let target = url
        .query_pairs()
        .find(|(k, _)| k == "url")
        .map(|(_, v)| v.into_owned());
    match target.as_deref().map(parse_origin) {
        Some(Ok(origin)) => {
            let enc: String =
                tauri::Url::parse_with_params("http://x/", &[("url", origin.as_str())])
                    .map(|u| u.query().unwrap_or_default().to_owned())
                    .unwrap_or_default();
            show_picker(app, &format!("?{enc}"));
        }
        _ => show_picker(app, "?change=1"),
    }
}

#[tauri::command]
fn get_server(state: tauri::State<State>) -> Option<String> {
    state.saved.lock().unwrap().server.clone()
}

#[tauri::command]
async fn set_server(app: AppHandle, url: String) -> Result<(), String> {
    let origin = parse_origin(&url)?;
    let check = origin.clone();
    tauri::async_runtime::spawn_blocking(move || check_server(&check))
        .await
        .map_err(|e| e.to_string())??;
    let origin_str = origin.origin().ascii_serialization();
    {
        let state = app.state::<State>();
        let mut saved = state.saved.lock().unwrap();
        if saved.server.as_deref() != Some(&origin_str) {
            saved.idp_hosts.clear();
        }
        saved.server = Some(origin_str.clone());
        drop(saved);
        state.persist();
    }
    navigate(&app, &format!("{origin_str}/"));
    Ok(())
}

/// Navigation policy: bundled page, the configured server, and the IdP it redirects to.
/// Anything else opens in the system browser.
fn allow_navigation(app: &AppHandle, url: &Url) -> bool {
    if is_local(url) {
        return true;
    }
    if url.scheme() == "corntchap" {
        handle_deep_link(app, url);
        return false;
    }
    let state = app.state::<State>();
    let server = state.saved.lock().unwrap().server.clone();
    let origin = origin_of(url);
    if origin.is_some() && origin == server {
        let mut until = state.login_until.lock().unwrap();
        if url.path().starts_with("/auth/oidc/login") {
            *until = Some(Instant::now() + LOGIN_WINDOW);
        } else if url.path().starts_with("/auth/oidc/callback") {
            *until = None;
        }
        return true;
    }
    if let Some(host) = url.host_str() {
        if url.scheme() == "https" {
            let in_login = state
                .login_until
                .lock()
                .unwrap()
                .is_some_and(|t| Instant::now() < t);
            let known = state.saved.lock().unwrap().idp_hosts.contains(host);
            if known {
                return true;
            }
            if in_login {
                // The sign-in redirect chain: remember the IdP so a later sign-in needs no window.
                state
                    .saved
                    .lock()
                    .unwrap()
                    .idp_hosts
                    .insert(host.to_owned());
                state.persist();
                return true;
            }
        }
    }
    if matches!(url.scheme(), "http" | "https" | "mailto" | "tel") {
        let _ = app.opener().open_url(url.as_str(), None::<&str>);
    }
    false
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .invoke_handler(tauri::generate_handler![get_server, set_server])
        .setup(|app| {
            let path = app.path().app_config_dir()?.join(CONFIG_FILE);
            let saved = std::fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Saved>(&b).ok())
                .unwrap_or_default();
            app.manage(State {
                path,
                saved: Mutex::new(saved),
                login_until: Mutex::new(None),
            });

            let nav = app.handle().clone();
            let new_win = app.handle().clone();
            let builder =
                WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App("index.html".into()))
                    .title("corntchap")
                    .background_color(Color(251, 245, 233, 255))
                    .initialization_script(INIT_SCRIPT)
                    .on_navigation(move |url| allow_navigation(&nav, url))
                    .on_new_window(move |url, _features| {
                        // target=_blank and window.open: same policy, never a second webview.
                        if allow_navigation(&new_win, &url) {
                            navigate(&new_win, url.as_str());
                        }
                        tauri::webview::NewWindowResponse::Deny
                    });
            #[cfg(desktop)]
            let builder = builder
                .inner_size(480.0, 800.0)
                .min_inner_size(360.0, 480.0);
            builder.build()?;

            let handle = app.handle().clone();
            #[cfg(desktop)]
            {
                let _ = app.deep_link().register("corntchap");
            }
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                for u in &urls {
                    handle_deep_link(&handle, u);
                }
            }
            let h = handle.clone();
            app.deep_link().on_open_url(move |event| {
                for u in event.urls() {
                    handle_deep_link(&h, &u);
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running corntchap");
}
