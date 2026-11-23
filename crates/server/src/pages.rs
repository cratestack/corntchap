//! The few server-rendered pages (sign-in landing, device approval, logout, errors): plain HTML and CSS in the
//! console's design (docs/design.md): an indigo left panel with the wax-print band and the pot logo, content on the
//! right. Fonts are the same three families the console uses, served from the binary at `/static/fonts/`.

use axum::response::Html;

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The font files embedded in the binary (latin subsets; the CSS falls back to system fonts for the rest).
pub fn font(name: &str) -> Option<&'static [u8]> {
    match name {
        "bricolage.woff2" => Some(include_bytes!("../assets/fonts/bricolage.woff2")),
        "inter.woff2" => Some(include_bytes!("../assets/fonts/inter.woff2")),
        "space-mono.woff2" => Some(include_bytes!("../assets/fonts/space-mono.woff2")),
        _ => None,
    }
}

const LOGO: &str = r##"<svg viewBox="0 0 24 24" width="30" height="30" role="img" aria-label="corntchap"><path d="M4.5 12.5h15v2.4a5.6 5.6 0 0 1-5.6 5.6h-3.8a5.6 5.6 0 0 1-5.6-5.6z" fill="#c8502c"/><path d="M3.2 12.5h17.6" stroke="#fbf5e9" stroke-width="1.6" stroke-linecap="round"/><path d="M2.6 13.6h1.9M19.5 13.6h1.9" stroke="#c8502c" stroke-width="1.6" stroke-linecap="round"/><g fill="none" stroke="#f2b33d" stroke-width="1.5" stroke-linecap="round"><path d="M8.6 10.1c-1.1-1.1 1.1-1.8 0-3"/><path d="M12 10.1c-1.1-1.1 1.1-1.8 0-3"/><path d="M15.4 10.1c-1.1-1.1 1.1-1.8 0-3"/></g></svg>"##;

const WAX: &str = r##"<svg class="wax" aria-hidden="true" focusable="false" height="64" preserveAspectRatio="xMinYMid slice"><defs><pattern id="wc" width="44" height="44" patternUnits="userSpaceOnUse"><circle cx="22" cy="22" r="20" fill="#c8502c"/><circle cx="22" cy="22" r="15" fill="#fbf5e9"/><circle cx="22" cy="22" r="10.5" fill="#1f7a72"/><circle cx="22" cy="22" r="6" fill="#f2b33d"/><circle cx="22" cy="22" r="2.2" fill="#15183a"/></pattern><pattern id="wz" width="22" height="12" patternUnits="userSpaceOnUse"><path d="M0 9 L5.5 3 L11 9 L16.5 3 L22 9" fill="none" stroke="#f2b33d" stroke-width="2" stroke-linejoin="round"/></pattern></defs><rect width="100%" height="44" fill="url(#wc)"/><rect y="48" width="100%" height="12" fill="url(#wz)"/></svg>"##;

const CSS: &str = r#"
@font-face{font-family:"Bricolage Grotesque";src:url(/static/fonts/bricolage.woff2) format("woff2");font-weight:200 800;font-display:swap}
@font-face{font-family:"Inter";src:url(/static/fonts/inter.woff2) format("woff2");font-weight:100 900;font-display:swap}
@font-face{font-family:"Space Mono";src:url(/static/fonts/space-mono.woff2) format("woff2");font-weight:400;font-display:swap}
:root{color-scheme:light dark;--bg:#fbf5e9;--card:#fffaf0;--muted:#f1e7d3;--border:#e7dcc6;--fg:#16130f;--fg2:#6a5f53;--pri:#c8502c;--ink:#16130f;--prifg:#fffaf0;--corn:#f2b33d;--ok:#1f7a72;--bad:#9b2f17;--side:#15183a;--shadow:2px 2px 0 0 var(--ink);--pb:var(--ink)}
@media (prefers-color-scheme:dark){:root{--bg:#0f1129;--card:#161a3d;--muted:#1d2148;--border:#262a55;--fg:#f3ecdd;--fg2:#a9acd0;--pri:#e0663f;--ink:#f3ecdd;--prifg:#160d09;--ok:#3fb3a4;--bad:#f0605a;--side:#0a0c1f;--shadow:none;--pb:transparent}}
*{box-sizing:border-box}
html{-webkit-font-smoothing:antialiased}
body{margin:0;min-height:100dvh;background:var(--bg);color:var(--fg);font:14px/1.6 Inter,ui-sans-serif,system-ui,sans-serif;display:grid;grid-template-columns:minmax(0,1fr)}
@media (min-width:860px){body{grid-template-columns:minmax(340px,5fr) 7fr}}
.side{background:var(--side);color:#f3ecdd;display:flex;flex-direction:column;min-height:0}
.side .wax{width:100%;display:block}
.side .in{padding:28px 32px 28px;display:flex;flex-direction:column;gap:18px;flex:1}
.brand{display:flex;align-items:center;gap:10px;font:800 20px/1 "Bricolage Grotesque",Inter,sans-serif;letter-spacing:-.015em}
.side h1{font:800 clamp(30px,4.4vw,46px)/1.04 "Bricolage Grotesque",Inter,sans-serif;letter-spacing:-.02em;margin:18px 0 0}
.side p{color:#a9acd0;margin:0;max-width:30ch;font-size:15px}
.who{margin-top:auto;padding-top:20px;border-top:1px solid #2b3066;font-size:13px;color:#a9acd0;display:flex;gap:6px;align-items:center;flex-wrap:wrap}
.who b{color:#f3ecdd}
.who form{display:inline;margin:0}
.who button{all:unset;color:var(--corn);text-decoration:underline;text-underline-offset:2px;cursor:pointer;font-size:13px}
.who button:focus-visible{outline:2px solid var(--corn);outline-offset:2px}
.main{display:grid;place-items:center;padding:40px 24px}
.card{width:100%;max-width:440px}
.main h1{font:800 30px/1.1 "Bricolage Grotesque",Inter,sans-serif;letter-spacing:-.015em;margin:0 0 10px}
.main p{margin:0 0 14px;color:var(--fg2)}
.label{font:400 10.5px/1 "Space Mono",ui-monospace,monospace;letter-spacing:.08em;text-transform:uppercase;color:var(--fg2)}
.code{font:400 46px/1.1 "Space Mono",ui-monospace,monospace;letter-spacing:.08em;margin:8px 0 18px;word-break:break-all}
.details{background:var(--card);border:1px solid var(--border);border-radius:12px;padding:4px 16px;margin:0 0 16px}
.details div{display:flex;justify-content:space-between;gap:16px;padding:10px 0;border-bottom:1px solid var(--border)}
.details div:last-child{border-bottom:0}
.details dt{color:var(--fg2)}.details dd{margin:0;font-weight:600;text-align:right;overflow-wrap:anywhere}
.warn{background:color-mix(in oklab,var(--corn) 18%,transparent);border:1px solid color-mix(in oklab,var(--corn) 60%,transparent);border-radius:12px;padding:10px 14px;margin:0 0 16px;color:var(--fg)}
.actions{display:flex;gap:12px;flex-wrap:wrap;margin-top:4px}
button,.btn{font:600 14px Inter,sans-serif;border-radius:12px;padding:11px 22px;cursor:pointer;text-decoration:none;display:inline-block;border:1.5px solid var(--border);background:var(--card);color:var(--fg)}
button.primary,.btn.primary{background:var(--pri);color:var(--prifg);border-color:var(--pb);box-shadow:var(--shadow)}
button.primary:active,.btn.primary:active{transform:translate(1px,1px);box-shadow:none}
button:focus-visible,.btn:focus-visible,input:focus-visible{outline:2px solid var(--fg);outline-offset:2px}
input[type=text],input:not([type]){font:400 28px "Space Mono",ui-monospace,monospace;letter-spacing:.08em;width:100%;padding:10px 14px;border:1px solid var(--border);border-radius:12px;background:var(--card);color:var(--fg);text-transform:uppercase;margin:6px 0 16px}
code{font:12.5px "Space Mono",ui-monospace,monospace;background:var(--muted);padding:1px 6px;border-radius:5px}
.choices{display:flex;flex-direction:column;gap:6px;margin:8px 0 16px}
.choice{display:flex;align-items:center;gap:8px;padding:8px 12px;border:1px solid var(--border);border-radius:10px;background:var(--card);cursor:pointer}
.choice:has(input:checked){border-color:var(--pri)}
.choice small{margin-left:auto;text-align:right}
.newcook{display:flex;gap:8px;align-items:center;flex-wrap:wrap}
.newcook input.plain{font:14px "Space Mono",ui-monospace,monospace;letter-spacing:0;text-transform:none;margin:0;padding:6px 10px;flex:1;min-width:10ch;width:auto;border-radius:8px}
select{font:13px Inter,sans-serif;padding:6px 8px;border:1px solid var(--border);border-radius:8px;background:var(--card);color:var(--fg)}
small{color:var(--fg2)}
.ok{color:var(--ok)}.bad{color:var(--bad)}
/* phones: a short indigo band, the form first; full-width actions, 48px targets, safe areas */
@media (max-width:859px){
body{grid-template-rows:auto 1fr}
.side .in{padding:18px 20px 20px;padding-top:max(18px,env(safe-area-inset-top));gap:10px}
.side h1{font-size:26px;margin-top:6px}
.side p{font-size:14px;max-width:none}
.who{padding-top:12px}
.main{place-items:start center;padding:28px 20px;padding-bottom:max(28px,env(safe-area-inset-bottom))}
.main h1{font-size:26px}
.code{font-size:34px}
.actions{flex-direction:column;align-items:stretch}
.actions form{display:flex;flex-direction:column}
button,.btn{min-height:48px;text-align:center;font-size:15px}
.actions>button,.actions>.btn,.actions form button{width:100%}
.choice{min-height:48px}
select,.newcook input.plain{min-height:44px;font-size:16px}
input[type=text],input:not([type]){font-size:24px}
}
@media (prefers-reduced-motion:reduce){*{transition:none!important}}
"#;

pub struct Left<'a> {
    pub headline: &'a str,
    pub sub: &'a str,
    /// (who as escaped HTML, see [`person`]; csrf) of the signed-in user, for "Signed in as X . Log out".
    pub who: Option<(String, &'a str)>,
}

/// A person for humans, as escaped HTML: "Display Name · @handle", or just "@handle" without a display name.
/// The display name comes from the identity provider and is untrusted: it is escaped here, like everything else.
pub fn person(handle: &str, display_name: Option<&str>) -> String {
    match display_name.filter(|d| !d.is_empty()) {
        Some(d) => format!("{} &middot; @{}", esc(d), esc(handle)),
        None => format!("@{}", esc(handle)),
    }
}

pub fn render(title: &str, left: Left<'_>, right: &str) -> Html<String> {
    let who = left
        .who
        .map(|(name, csrf)| {
            format!(
                "<div class=who>Signed in as <b>{}</b> &middot; <form method=post action=/logout><input type=hidden name=csrf value=\"{}\"><button>Log out</button></form></div>",
                name,
                esc(csrf)
            )
        })
        .unwrap_or_default();
    Html(format!(
        "<!doctype html><html lang=en><head><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1,viewport-fit=cover\"><meta name=theme-color content=\"#15183a\"><meta name=color-scheme content=\"light dark\"><title>{title} · corntchap</title><style>{CSS}</style></head><body>\
         <aside class=side>{WAX}<div class=in><div class=brand>{LOGO}<span>corntchap</span></div><h1>{headline}</h1><p>{sub}</p>{who}</div></aside>\
         <main class=main><div class=card>{right}</div></main></body></html>",
        title = esc(title),
        headline = left.headline,
        sub = left.sub,
    ))
}

/// A generic page (errors, notices): headline on the left, the given HTML on the right.
pub fn simple(title: &str, body: &str) -> Html<String> {
    render(
        title,
        Left {
            headline: "A shared pot for your coding agents",
            sub: "Claim a task, tick the checklist, serve it. One lease at a time.",
            who: None,
        },
        body,
    )
}
