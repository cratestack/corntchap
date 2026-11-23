import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { AlertCircle, ArrowRight, Loader2, Server, X } from "lucide-react";
import { LogoMark, WaxBand, Wordmark } from "@console/components/brand";
import { Button } from "@console/components/ui/button";
import { Input } from "@console/components/ui/input";
import { Label } from "@console/components/ui/label";
import { getServer, setServer } from "./tauri";
import { DEFAULT_URL, forgetServer, hostOf, loadRecent, normalise, rememberServer } from "./servers";

type Phase =
  | { kind: "boot" }
  | { kind: "form" }
  /** Saved server on launch: a short "opening" state with a way out, then straight to the console. */
  | { kind: "opening"; origin: string }
  | { kind: "connecting"; origin: string };

/** Query the Rust shell puts on the picker URL: `?url=` (deep link, prefill) or `?change=1`. */
function readQuery() {
  const q = new URLSearchParams(location.search);
  return { link: normalise(q.get("url")), change: q.has("change") };
}

export function App() {
  const [phase, setPhase] = useState<Phase>({ kind: "boot" });
  const [value, setValue] = useState(DEFAULT_URL);
  const [error, setError] = useState<string | null>(null);
  const [fromLink, setFromLink] = useState(false);
  const [recent, setRecent] = useState<string[]>(() => loadRecent());
  const inputRef = useRef<HTMLInputElement>(null);
  const autoTimer = useRef<number | undefined>(undefined);

  const connect = useCallback(async (origin: string) => {
    setError(null);
    setPhase({ kind: "connecting", origin });
    try {
      // On success Rust navigates this webview away to <origin>/, so there is nothing to do after it.
      await setServer(origin);
      rememberServer(origin);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setValue(origin);
      setPhase({ kind: "form" });
    }
  }, []);

  useEffect(() => {
    let live = true;
    void (async () => {
      const { link, change } = readQuery();
      const saved = await getServer();
      if (!live) return;
      if (link) {
        setValue(link);
        setFromLink(true);
        setPhase({ kind: "form" });
      } else if (saved && !change) {
        setValue(saved);
        setPhase({ kind: "opening", origin: saved });
        autoTimer.current = window.setTimeout(() => void connect(saved), 700);
      } else {
        setValue(saved ?? DEFAULT_URL);
        setPhase({ kind: "form" });
      }
    })();
    return () => {
      live = false;
      window.clearTimeout(autoTimer.current);
    };
  }, [connect]);

  const changeServer = () => {
    window.clearTimeout(autoTimer.current);
    setPhase({ kind: "form" });
  };

  const submit = (e: FormEvent) => {
    e.preventDefault();
    const origin = normalise(value);
    if (!origin) {
      setError(`Enter a full address, for example ${DEFAULT_URL}`);
      inputRef.current?.focus();
      return;
    }
    void connect(origin);
  };

  const busy = phase.kind === "opening" || phase.kind === "connecting";

  return (
    <div className="flex min-h-dvh flex-col bg-background text-foreground">
      <div className="pt-[env(safe-area-inset-top)]">
        <WaxBand height={64} />
      </div>

      <main className="mx-auto flex w-full max-w-md flex-1 flex-col px-4 pt-8 pb-[max(1.5rem,env(safe-area-inset-bottom))] pl-[max(1rem,env(safe-area-inset-left))] pr-[max(1rem,env(safe-area-inset-right))]">
        <header className="flex flex-col items-center text-center">
          <LogoMark className="size-16 drop-shadow-sm" />
          <h1 className="mt-3 text-[28px] leading-none">
            <Wordmark className="text-[28px]" />
          </h1>
          <p className="mt-2 text-[15px] text-muted-foreground">
            {fromLink && phase.kind === "form"
              ? "Switch to this server?"
              : "Your coding agents, on your phone."}
          </p>
        </header>

        {busy ? (
          <Connecting
            origin={phase.origin}
            checking={phase.kind === "connecting"}
            onChange={changeServer}
          />
        ) : phase.kind === "boot" ? null : (
          <form onSubmit={submit} noValidate className="mt-8 flex flex-col gap-3">
            <Label htmlFor="server-url" className="text-[13px] text-muted-foreground">
              Server address
            </Label>
            <Input
              ref={inputRef}
              id="server-url"
              name="url"
              type="url"
              inputMode="url"
              autoCapitalize="none"
              autoComplete="url"
              autoCorrect="off"
              spellCheck={false}
              autoFocus
              value={value}
              placeholder={DEFAULT_URL}
              aria-invalid={error ? true : undefined}
              aria-describedby={error ? "server-error" : "server-hint"}
              onChange={(e) => {
                setValue(e.target.value);
                if (error) setError(null);
              }}
              className="h-12 rounded-xl px-3.5 text-base"
            />

            {error ? (
              <p
                id="server-error"
                role="alert"
                className="flex items-start gap-2 rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2.5 text-[13.5px] leading-snug text-destructive [overflow-wrap:anywhere]"
              >
                <AlertCircle className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
                <span>{error}</span>
              </p>
            ) : (
              <p id="server-hint" className="text-[13px] text-muted-foreground">
                Each organisation runs its own corntchap server. Ask your admin for its address.
              </p>
            )}

            <Button type="submit" size="lg" className="mt-1 h-12 w-full rounded-xl text-base">
              Connect
              <ArrowRight aria-hidden="true" />
            </Button>

            {recent.length > 0 && (
              <section aria-label="Recent servers" className="mt-5">
                <h2 className="label-caps mb-2 font-mono! text-[10.5px] font-normal tracking-[0.08em]">
                  Recent servers
                </h2>
                <ul className="flex flex-col gap-2">
                  {recent.map((origin) => (
                    <li key={origin} className="flex items-stretch gap-2">
                      <button
                        type="button"
                        onClick={() => void connect(origin)}
                        className="flex min-h-12 min-w-0 flex-1 items-center gap-3 rounded-xl border border-border bg-card px-3.5 text-left transition-colors active:bg-accent"
                      >
                        <Server className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                        <span className="min-w-0 flex-1 truncate font-mono text-[13px]">{hostOf(origin)}</span>
                      </button>
                      <button
                        type="button"
                        aria-label={`Forget ${hostOf(origin)}`}
                        onClick={() => setRecent(forgetServer(origin))}
                        className="grid size-12 shrink-0 place-items-center rounded-xl border border-border bg-card text-muted-foreground active:bg-accent"
                      >
                        <X className="size-4" aria-hidden="true" />
                      </button>
                    </li>
                  ))}
                </ul>
              </section>
            )}
          </form>
        )}

        <p className="mt-auto pt-8 text-center text-[12px] text-muted-foreground">
          Hold three fingers on the screen to return here.
        </p>
      </main>
    </div>
  );
}

/** Screen (b): shown while the server is checked and the console is about to open. */
function Connecting({
  origin,
  checking,
  onChange,
}: {
  origin: string;
  checking: boolean;
  onChange: () => void;
}) {
  return (
    <section
      aria-live="polite"
      aria-busy="true"
      className="mt-8 flex flex-col items-center gap-4 rounded-2xl border border-border bg-card px-5 py-8 text-center"
    >
      <Loader2 className="size-7 animate-spin text-primary motion-reduce:animate-none" aria-hidden="true" />
      <div className="min-w-0 max-w-full">
        <p className="font-display text-[19px] leading-tight">{checking ? "Connecting…" : "Opening your console…"}</p>
        <p className="mt-1.5 truncate font-mono text-[13px] text-muted-foreground">{hostOf(origin)}</p>
      </div>
      <Button variant="outline" size="lg" className="h-12 w-full rounded-xl" onClick={onChange} disabled={checking}>
        Change server
      </Button>
    </section>
  );
}
