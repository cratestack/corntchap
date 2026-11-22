import { useState } from "react";
import { Check, Copy, KeyRound, Loader2, RotateCw, TriangleAlert } from "lucide-react";
import { toast } from "sonner";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { errorMessage } from "@/lib/api";
import { useV1Call } from "@/lib/data";
import { copyText, mcpSnippets, type CookKind } from "@/lib/snippets";

export function CopyButton({ text, label = "Copy", className, size = "sm" }: { text: string; label?: string; className?: string; size?: "sm" | "xs" }) {
  const [done, setDone] = useState(false);
  return (
    <Button
      type="button"
      variant="outline"
      size={size}
      className={className}
      onClick={async () => {
        if (await copyText(text)) {
          setDone(true);
          toast.success("Copied to the clipboard");
          setTimeout(() => setDone(false), 1800);
        } else toast.error("The browser refused to copy. Select the text and copy it by hand.");
      }}
    >
      {done ? <Check /> : <Copy />} {done ? "Copied" : label}
    </Button>
  );
}

export function Snippets({ token, defaultKind }: { token?: string | null; defaultKind?: string | null }) {
  const snippets = mcpSnippets(window.location.origin, token);
  const start = (defaultKind && snippets.find((s) => s.id === (defaultKind as CookKind))?.id) || "claude-code";
  const [active, setActive] = useState<string>(start);
  const cur = snippets.find((s) => s.id === active) ?? snippets[0]!;
  return (
    <div className="min-w-0 max-w-full">
      <div role="tablist" aria-label="MCP client" className="scrollbar-thin flex gap-1 overflow-x-auto rounded-lg bg-muted p-1">
        {snippets.map((s) => (
          <button
            key={s.id}
            type="button"
            role="tab"
            aria-selected={active === s.id}
            onClick={() => setActive(s.id)}
            className={cn(
              "flex-1 whitespace-nowrap rounded-md px-2 py-1.5 text-[12.5px] font-medium text-muted-foreground transition-colors",
              active === s.id && "bg-card text-foreground shadow-sm",
            )}
          >
            {s.title}
          </button>
        ))}
      </div>
      <div className="mt-3 flex min-w-0 items-center justify-between gap-2">
        <span className="min-w-0 truncate font-mono text-[11.5px] text-muted-foreground">{cur.file}</span>
        <CopyButton text={cur.body} label="Copy snippet" />
      </div>
      <pre
        tabIndex={0}
        aria-label={`${cur.title} MCP configuration`}
        className="scrollbar-thin mt-2 max-h-72 w-full max-w-full overflow-auto rounded-lg border border-border bg-muted/60 p-3.5 font-mono text-[12px] leading-relaxed"
      >
        {cur.body}
      </pre>
      {!token && (
        <p className="mt-2 text-[12px] text-muted-foreground">
          <code className="font-mono">&lt;token&gt;</code> stands for the cook's own token. It is shown once, when the cook is made or its token rotated.
        </p>
      )}
    </div>
  );
}

export function McpDialog({
  open,
  onOpenChange,
  name,
  kind,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  name: string;
  kind: string | null;
}) {
  const call = useV1Call();
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [token, setToken] = useState<string | null>(null);

  function change(o: boolean) {
    if (!o) {
      // the token is shown once: closing the dialog forgets it
      setToken(null);
      setConfirming(false);
    }
    onOpenChange(o);
  }

  async function rotate() {
    setBusy(true);
    try {
      const r = await call<{ token: string }>("POST", `/agents/${name}/rotate`);
      setToken(r.token);
      setConfirming(false);
    } catch (e) {
      toast.error("Could not rotate the token", { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={change}>
      <DialogContent className="max-h-[92dvh] overflow-y-auto sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>MCP config for {name}</DialogTitle>
          <DialogDescription>The same snippets as <code className="font-mono">corntchap-cli cook setup</code>. Pick the client this cook runs in.</DialogDescription>
        </DialogHeader>
        {token ? (
          <>
            <div className="min-w-0 space-y-2">
              <div className="flex items-center gap-2 rounded-lg border border-corn/60 bg-corn/10 p-2.5">
                <KeyRound className="size-4 shrink-0 text-corn-text" aria-hidden="true" />
                <code className="min-w-0 flex-1 break-all font-mono text-[12.5px]" data-testid="token">
                  {token}
                </code>
                <CopyButton text={token} label="Copy token" />
              </div>
              <p className="flex items-start gap-1.5 text-[12.5px] text-muted-foreground" role="status">
                <TriangleAlert className="mt-px size-3.5 shrink-0 text-corn-text" aria-hidden="true" />
                <span>
                  This is {name}'s new token, shown only now. The old token stopped working the moment it was made, so update every place the cook runs.
                </span>
              </p>
            </div>
            <Snippets token={token} defaultKind={kind} />
          </>
        ) : (
          <>
            <div className="min-w-0 space-y-1.5 rounded-lg border border-primary/40 bg-primary/5 px-3 py-2.5 text-[12.5px]">
              <p>
                <b>No token needed:</b> point the repo's MCP config at a pot&rsquo;s URL and commit it. The agent signs in with OAuth on first use
                and you pick the cook it works as.
              </p>
              <div className="flex min-w-0 items-center gap-2">
                <code className="min-w-0 flex-1 truncate font-mono text-[12px]">{`${window.location.origin}/mcp/<POT>`}</code>
                <CopyButton text={`${window.location.origin}/mcp/`} label="Copy the MCP URL" />
              </div>
            </div>
            <p className="rounded-lg border border-border bg-muted/40 px-3 py-2 text-[12.5px]">
              Or with a token: use the cook's token, not a fire (relay) token. A relay token only wakes cooks; it cannot scoop tasks over MCP.
            </p>
            <Snippets defaultKind={kind} />
            <div className="min-w-0 rounded-lg border border-border bg-card p-3">
              {confirming ? (
                <div className="space-y-2.5">
                  <p className="text-[12.5px]">
                    Rotating makes a new token for {name} and <b>the old one stops working immediately</b>. Anything using it must be updated.
                  </p>
                  <div className="flex flex-wrap gap-2">
                    <Button size="sm" disabled={busy} onClick={() => void rotate()}>
                      {busy ? <Loader2 className="animate-spin" /> : <RotateCw />} Rotate it now
                    </Button>
                    <Button size="sm" variant="ghost" disabled={busy} onClick={() => setConfirming(false)}>
                      Not now
                    </Button>
                  </div>
                </div>
              ) : (
                <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
                  <p className="min-w-0 flex-1 basis-60 text-[12.5px] text-muted-foreground">
                    Do not have the token any more? Make a new one and see these snippets with it filled in.
                  </p>
                  <Button size="sm" variant="outline" onClick={() => setConfirming(true)}>
                    <RotateCw /> Rotate token and fill in
                  </Button>
                </div>
              )}
            </div>
          </>
        )}
        {token && (
          <DialogFooter>
            <Button onClick={() => change(false)}>I have copied it</Button>
          </DialogFooter>
        )}
      </DialogContent>
    </Dialog>
  );
}

/** Shows a freshly minted token exactly once, with the snippets already filled in. */
export function TokenDialog({
  open,
  onOpenChange,
  title,
  name,
  token,
  kind,
  hint,
  withSnippets = true,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  title: string;
  name: string;
  token: string | null;
  kind: string | null;
  hint?: string;
  withSnippets?: boolean;
}) {
  return (
    <Dialog open={open} onOpenChange={(o) => onOpenChange(o)}>
      <DialogContent className="max-h-[92dvh] overflow-y-auto sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            This is the only time {name}'s token is shown. Copy it now; the server keeps just a hash.
          </DialogDescription>
        </DialogHeader>
        {token && (
          <>
            <div className="flex items-center gap-2 rounded-lg border border-corn/60 bg-corn/10 p-2.5">
              <KeyRound className="size-4 shrink-0 text-corn-text" aria-hidden="true" />
              <code className="min-w-0 flex-1 break-all font-mono text-[12.5px]" data-testid="token">
                {token}
              </code>
              <CopyButton text={token} label="Copy token" />
            </div>
            {hint && <p className="text-[12.5px] text-muted-foreground">{hint}</p>}
            {withSnippets && <Snippets token={token} defaultKind={kind} />}
          </>
        )}
        <DialogFooter>
          <Button onClick={() => onOpenChange(false)}>I have copied it</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
