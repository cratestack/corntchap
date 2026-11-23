/** IPC to the Rust shell. Only the bundled page (this app) has it; the console served from the server does not. */
type Invoke = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

function internals(): Invoke | null {
  const w = window as unknown as { __TAURI_INTERNALS__?: { invoke: Invoke } };
  return w.__TAURI_INTERNALS__?.invoke ?? null;
}

/** The saved server origin, or null (first launch, or opened in a plain browser). */
export async function getServer(): Promise<string | null> {
  const invoke = internals();
  if (!invoke) return null;
  try {
    return ((await invoke("get_server")) as string | null) ?? null;
  } catch {
    return null;
  }
}

/** Rust checks `<origin>/healthz` natively (no CORS), saves the origin and navigates the webview. Rejects with a message. */
export async function setServer(url: string): Promise<void> {
  const invoke = internals();
  if (!invoke) throw new Error("This page only works inside the corntchap app.");
  try {
    await invoke("set_server", { url });
  } catch (e) {
    throw new Error(typeof e === "string" ? e : e instanceof Error ? e.message : "Something went wrong.");
  }
}
