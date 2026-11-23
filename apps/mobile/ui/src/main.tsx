import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import { App } from "./app";

// The console toggles `.dark` on <html> (its tokens key off it). Here it simply follows the system.
const dark = matchMedia("(prefers-color-scheme: dark)");
const sync = () => document.documentElement.classList.toggle("dark", dark.matches);
sync();
dark.addEventListener("change", sync);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
