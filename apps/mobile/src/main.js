// Server picker. Runs on the bundled local origin only, which is the one origin that has IPC.
// Flow: ask Rust for the saved server. With one saved (and no ?change), show a short "opening" state
// with a way out, then Rust navigates the webview to <server>/. Otherwise show the form.
const DEFAULT_URL = "https://corntchap.camer.digital";
const $ = (id) => document.getElementById(id);
const invoke = (cmd, args) => window.__TAURI_INTERNALS__.invoke(cmd, args);

const form = $("form"), input = $("url"), status = $("status"), go = $("go"), lede = $("lede");

function setStatus(text, err) {
  status.textContent = text || "";
  status.className = err ? "err" : "";
}

function normalise(raw) {
  let v = (raw || "").trim();
  if (!v) return null;
  if (!/^[a-z][a-z0-9+.-]*:\/\//i.test(v)) v = "https://" + v;
  try {
    const u = new URL(v);
    if (u.protocol !== "https:" && u.protocol !== "http:") return null;
    return u.origin; // the console lives at the root of the origin
  } catch {
    return null;
  }
}

// Rust checks <origin>/healthz natively (no CORS) and refuses to save a server that does not answer.
async function connect(origin) {
  go.disabled = true;
  setStatus("Checking " + origin + " ...");
  try {
    await invoke("set_server", { url: origin });
  } catch (e) {
    setStatus(String(e), true);
    go.disabled = false;
    go.type = "submit";
    go.textContent = "Connect";
  }
}

form.addEventListener("submit", (e) => {
  e.preventDefault();
  const origin = normalise(input.value);
  if (!origin) return setStatus("Enter a full address, e.g. " + DEFAULT_URL, true);
  connect(origin);
});

(async () => {
  const q = new URLSearchParams(location.search);
  let saved = null;
  try { saved = await invoke("get_server"); } catch { /* opened in a plain browser */ }
  const fromLink = normalise(q.get("url"));
  input.value = fromLink || saved || DEFAULT_URL;
  if (fromLink) {
    lede.textContent = "Switch to this server?";
  } else if (saved && !q.has("change")) {
    lede.textContent = "Opening your console ...";
    setStatus(saved);
    go.textContent = "Change server";
    go.type = "button";
    const timer = setTimeout(() => connect(saved), 700);
    go.addEventListener("click", () => {
      clearTimeout(timer);
      location.replace("index.html?change=1");
    }, { once: true });
    return;
  } else if (saved) {
    lede.textContent = "Change server";
  }
  input.focus({ preventScroll: true });
})();
