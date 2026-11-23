import { useEffect, useRef, useState, type ReactNode } from "react";
import { Check, Copy, KeyRound, Loader2, RotateCw, TriangleAlert } from "lucide-react";
import { toast } from "sonner";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { errorMessage } from "@/lib/api";
import { useV1Call } from "@/lib/data";
import { copyText, mcpSnippets } from "@/lib/snippets";
import { FormSheetBody, FormSheetFoot, FormSheetHead, formSheetClass } from "./bits";

/**
 * Copies text and says so: the button turns into "Copied" for a moment (and a toast), so it works the same with a
 * thumb as with a mouse. `block` makes it a big full-width button (the phone's main action on a secret).
 */
export function CopyButton({
  text,
  label = "Copy",
  className,
  size = "sm",
  block,
  variant = "outline",
}: {
  text: string;
  label?: string;
  className?: string;
  size?: "sm" | "xs";
  block?: boolean;
  variant?: "outline" | "default";
}) {
  const [done, setDone] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  return (
    <Button
      type="button"
      variant={variant}
      size={block ? "default" : size}
      className={cn(block && "h-11 w-full text-[14px]", done && "border-success/50 text-success", className)}
      onClick={async () => {
        if (await copyText(text)) {
          setDone(true);
          toast.success("Copied to the clipboard");
          clearTimeout(timer.current);
          timer.current = setTimeout(() => setDone(false), 1800);
        } else toast.error("The browser refused to copy. Select the text and copy it by hand.");
      }}
    >
      {done ? <Check /> : <Copy />}
      <span aria-live="polite">{done ? "Copied" : label}</span>
    </Button>
  );
}

/** Code or a command: scrolls sideways inside its own box, never the page. */
export function CodeBlock({ children, label, className, wrap }: { children: string; label?: string; className?: string; wrap?: boolean }) {
  return (
    <pre
      tabIndex={0}
      aria-label={label}
      className={cn(
        "scrollbar-thin max-h-72 w-full min-w-0 max-w-full overflow-auto rounded-lg border border-border bg-muted/60 p-3.5 font-mono text-[12px] leading-relaxed",
        wrap ? "whitespace-pre-wrap break-words" : "whitespace-pre",
        className,
      )}
    >
      {children}
    </pre>
  );
}

/** One line to copy (a command, a URL): the text scrolls inside the box and the copy button sits under it on phones. */
export function CopyLine({ text, label = "Copy", className }: { text: string; label?: string; className?: string }) {
  return (
    <div className={cn("flex min-w-0 flex-col gap-2 rounded-lg bg-muted/50 p-2.5 sm:flex-row sm:items-center", className)}>
      <code className="scrollbar-thin min-w-0 flex-1 overflow-x-auto whitespace-pre py-1 font-mono text-[12px]">{text}</code>
      <CopyButton text={text} label={label} className="max-sm:h-11 max-sm:w-full" />
    </div>
  );
}

/** A token shown once: a saffron-edged box, the token wrapped, a big copy button. */
export function TokenBox({ token, name }: { token: string; name?: string }) {
  return (
    <div className="min-w-0 space-y-2.5 rounded-xl border border-corn/60 bg-corn/10 p-3">
      <div className="flex items-center gap-2">
        <KeyRound className="size-4 shrink-0 text-corn-text" aria-hidden="true" />
        <span className="label-caps">{name ? `${name}'s token` : "Token"}</span>
      </div>
      <code className="block break-all rounded-lg bg-background/70 px-3 py-2.5 font-mono text-[12.5px] leading-relaxed" data-testid="token">
        {token}
      </code>
      <CopyButton text={token} label="Copy token" block variant="default" />
    </div>
  );
}

export function Snippets({ token, defaultKind }: { token?: string | null; defaultKind?: string | null }) {
  const snippets = mcpSnippets(window.location.origin, token);
  const start = (defaultKind && snippets.find((s) => s.id === defaultKind)?.id) || "claude-code";
  const [active, setActive] = useState<string>(start);
  const cur = snippets.find((s) => s.id === active) ?? snippets[0]!;
  return (
    <div className="min-w-0 max-w-full">
      <div role="tablist" aria-label="MCP client" className="scrollbar-none flex gap-1 overflow-x-auto rounded-lg bg-muted p-1">
        {snippets.map((s) => (
          <button
            key={s.id}
            type="button"
            role="tab"
            aria-selected={active === s.id}
            onClick={() => setActive(s.id)}
            className={cn(
              "h-9 flex-1 whitespace-nowrap rounded-md px-3 text-[12.5px] font-medium text-muted-foreground transition-colors pointer-coarse:h-10",
              active === s.id && "bg-card text-foreground shadow-sm",
            )}
          >
            {s.title}
          </button>
        ))}
      </div>
      <p className="mt-3 truncate font-mono text-[11.5px] text-muted-foreground">{cur.file}</p>
      <CodeBlock label={`${cur.title} MCP configuration`} className="mt-1.5">
        {cur.body}
      </CodeBlock>
      <CopyButton text={cur.body} label="Copy snippet" className="mt-2 max-sm:h-11 max-sm:w-full" />
      {!token && (
        <p className="mt-2 text-[12px] text-muted-foreground">
          <code className="font-mono">&lt;token&gt;</code> stands for the cook's own token. It is shown once, when the cook is made or its token rotated.
        </p>
      )}
    </div>
  );
}

/** A dialog shell for secrets: a bottom sheet on phones with a scrolling body and a sticky footer. */
function SecretShell({
  open,
  onOpenChange,
  title,
  description,
  children,
  footer,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  title: string;
  description: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className={cn(formSheetClass, "sm:max-w-2xl")}>
        <FormSheetHead>
          <DialogTitle className="text-[22px] leading-tight">{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </FormSheetHead>
        <FormSheetBody className="pb-4">{children}</FormSheetBody>
        {footer && <FormSheetFoot>{footer}</FormSheetFoot>}
      </DialogContent>
    </Dialog>
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
  const url = `${window.location.origin}/mcp/`;

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
    <SecretShell
      open={open}
      onOpenChange={change}
      title={`MCP config for ${name}`}
      description={
        <>
          The same snippets as <code className="font-mono">corntchap-cli cook setup</code>. Pick the client this cook runs in.
        </>
      }
      footer={token ? <Button className="max-sm:h-11" onClick={() => change(false)}>I have copied it</Button> : undefined}
    >
      {token ? (
        <>
          <TokenBox token={token} name={name} />
          <p className="flex items-start gap-1.5 text-[12.5px] text-muted-foreground" role="status">
            <TriangleAlert className="mt-px size-3.5 shrink-0 text-corn-text" aria-hidden="true" />
            <span>This token is shown only now. The old one stopped working the moment it was made, so update every place the cook runs.</span>
          </p>
          <Snippets token={token} defaultKind={kind} />
        </>
      ) : (
        <>
          <div className="min-w-0 space-y-2 rounded-xl border border-primary/40 bg-primary/5 p-3 text-[12.5px]">
            <p>
              <b>No token needed:</b> point the repo's MCP config at a pot's URL and commit it. The agent signs in with OAuth on first use and you pick the
              cook it works as.
            </p>
            <CopyLine text={`${url}<POT>`} label="Copy the MCP URL" />
          </div>
          <p className="rounded-xl border border-border bg-muted/40 px-3 py-2.5 text-[12.5px]">
            Or with a token: use the cook's token, not a fire (relay) token. A relay token only wakes cooks; it cannot scoop tasks over MCP.
          </p>
          <Snippets defaultKind={kind} />
          <div className="min-w-0 rounded-xl border border-border bg-card p-3">
            {confirming ? (
              <div className="space-y-2.5">
                <p className="text-[12.5px]">
                  Rotating makes a new token for {name} and <b>the old one stops working immediately</b>. Anything using it must be updated.
                </p>
                <div className="flex flex-col gap-2 sm:flex-row">
                  <Button className="max-sm:h-11" disabled={busy} onClick={() => void rotate()}>
                    {busy ? <Loader2 className="animate-spin" /> : <RotateCw />} Rotate it now
                  </Button>
                  <Button className="max-sm:h-11" variant="ghost" disabled={busy} onClick={() => setConfirming(false)}>
                    Not now
                  </Button>
                </div>
              </div>
            ) : (
              <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
                <p className="min-w-0 flex-1 text-[12.5px] text-muted-foreground">Do not have the token any more? Make a new one and see these snippets with it filled in.</p>
                <Button variant="outline" className="max-sm:h-11" onClick={() => setConfirming(true)}>
                  <RotateCw /> Rotate token and fill in
                </Button>
              </div>
            )}
          </div>
        </>
      )}
    </SecretShell>
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
    <SecretShell
      open={open}
      onOpenChange={onOpenChange}
      title={title}
      description={`This is the only time ${name}'s token is shown. Copy it now; the server keeps just a hash.`}
      footer={<Button className="max-sm:h-11" onClick={() => onOpenChange(false)}>I have copied it</Button>}
    >
      {token && (
        <>
          <TokenBox token={token} />
          {hint && <p className="text-[12.5px] text-muted-foreground">{hint}</p>}
          {withSnippets && <Snippets token={token} defaultKind={kind} />}
        </>
      )}
    </SecretShell>
  );
}
