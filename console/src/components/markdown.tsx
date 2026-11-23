import ReactMarkdown, { defaultUrlTransform } from "react-markdown";
import remarkGfm from "remark-gfm";
import { cn } from "@/lib/utils";

/** `@name` outside code spans becomes a `mention:` link, rendered as a highlighted chip below. */
function markMentions(md: string): string {
  return md
    .split(/(```[\s\S]*?```|`[^`\n]*`)/)
    .map((part, i) => (i % 2 === 1 ? part : part.replace(/(^|[\s(])@([\w][\w.-]*)/g, (_m, pre: string, name: string) => `${pre}[@${name}](mention:${name})`)))
    .join("");
}

/** `SHOP-4` (for the given pot keys) outside code spans becomes a `task:` link, rendered as a chip that opens the task. */
function markTasks(md: string, keys: string[]): string {
  if (keys.length === 0) return md;
  const re = new RegExp(`(^|[^\\w\\[/-])((?:${keys.map((k) => k.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|")})-\\d+)\\b`, "g");
  return md
    .split(/(```[\s\S]*?```|`[^`\n]*`|\]\([^)]*\))/)
    .map((part, i) => (i % 2 === 1 ? part : part.replace(re, (_m, pre: string, id: string) => `${pre}[${id}](task:${id})`)))
    .join("");
}

/**
 * Markdown for task bodies and comments. Raw HTML is never rendered; links open in a new tab.
 * With `mentions` (your own handle), `@name` is highlighted, and yours and `@all` more strongly.
 */
export function Markdown({
  children,
  className,
  mentions,
  taskKeys,
  onTask,
}: {
  children: string;
  className?: string;
  mentions?: string;
  /** Pot keys whose task ids (`SHOP-4`) become chips; with `onTask` they open the task. */
  taskKeys?: string[];
  onTask?: (id: string) => void;
}) {
  return (
    <div className={cn("prose-ct", className)}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        urlTransform={(url) => (url.startsWith("mention:") || url.startsWith("task:") ? url : defaultUrlTransform(url))}
        components={{
          a: ({ node: _n, href, children: kids, ...props }) => {
            if (href?.startsWith("task:")) {
              const id = href.slice(5);
              return (
                <button
                  type="button"
                  onClick={() => onTask?.(id)}
                  title={`Open ${id}`}
                  className="mx-px inline-flex items-center rounded-md border border-border bg-background/70 px-1.5 py-px align-baseline font-mono text-[0.85em] font-bold tracking-tight text-foreground no-underline hover:border-foreground/40"
                >
                  {id}
                </button>
              );
            }
            if (href?.startsWith("mention:")) {
              const name = href.slice(8);
              const strong = name === mentions || /^(all|everyone)$/i.test(name);
              return (
                <span
                  className={cn(
                    "rounded-md px-0.5 py-px font-medium",
                    strong ? "bg-primary/15 text-primary" : "bg-muted text-foreground/80",
                  )}
                >
                  {kids}
                </span>
              );
            }
            return (
              <a href={href} {...props} target="_blank" rel="noreferrer noopener">
                {kids}
              </a>
            );
          },
        }}
      >
        {taskKeys ? markTasks(mentions !== undefined ? markMentions(children) : children, taskKeys) : mentions !== undefined ? markMentions(children) : children}
      </ReactMarkdown>
    </div>
  );
}
