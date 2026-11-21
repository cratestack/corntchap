// Runs before first paint so there is no light flash in dark mode. External file: the server's CSP forbids inline scripts.
(function () {
  try {
    var t = localStorage.getItem("corntchap.theme") || "system";
    var dark = t === "dark" || (t === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
    if (dark) document.documentElement.classList.add("dark");
    document.documentElement.style.colorScheme = dark ? "dark" : "light";
  } catch (e) {
    /* storage unavailable: stay light */
  }
})();
