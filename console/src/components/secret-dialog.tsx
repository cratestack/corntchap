import { useState } from "react";
import { Check, Copy, KeyRound } from "lucide-react";
import { toast } from "sonner";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
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
    <div>
      <div role="tablist" aria-label="MCP client" className="flex gap-1 rounded-lg bg-muted p-1">
        {snippets.map((s) => (
          <button
            key={s.id}
            type="button"
            role="tab"
            aria-selected={active === s.id}
            onClick={() => setActive(s.id)}
            className={cn(
              "flex-1 rounded-md px-2 py-1.5 text-[12.5px] font-medium text-muted-foreground transition-colors",
              active === s.id && "bg-card text-foreground shadow-sm",
            )}
          >
            {s.title}
          </button>
        ))}
      </div>
      <div className="mt-3 flex items-center justify-between gap-2">
        <span className="font-mono text-[11.5px] text-muted-foreground">{cur.file}</span>
        <CopyButton text={cur.body} label="Copy snippet" />
      </div>
      <pre
        tabIndex={0}
        aria-label={`${cur.title} MCP configuration`}
        className="scrollbar-thin mt-2 max-h-72 overflow-auto rounded-lg border border-border bg-muted/60 p-3.5 font-mono text-[12px] leading-relaxed"
      >
        {cur.body}
      </pre>
      {!token && (
        <p className="mt-2 text-[12px] text-muted-foreground">
          Replace <code className="font-mono">&lt;token&gt;</code> with the cook's token (shown once when it was made or rotated), or keep it in an env var.
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
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>MCP config for {name}</DialogTitle>
          <DialogDescription>The same snippets as <code className="font-mono">corntchap-cli cook setup</code>. Pick the client this cook runs in.</DialogDescription>
        </DialogHeader>
        <Snippets defaultKind={kind} />
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
      <DialogContent className="max-h-[92dvh] overflow-y-auto sm:max-w-xl">
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
