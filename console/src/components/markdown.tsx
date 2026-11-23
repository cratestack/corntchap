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

/**
 * Markdown for task bodies and comments. Raw HTML is never rendered; links open in a new tab.
 * With `mentions` (your own handle), `@name` is highlighted, and yours and `@all` more strongly.
 */
export function Markdown({ children, className, mentions }: { children: string; className?: string; mentions?: string }) {
  return (
    <div className={cn("prose-ct", className)}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        urlTransform={(url) => (url.startsWith("mention:") ? url : defaultUrlTransform(url))}
        components={{
          a: ({ node: _n, href, children: kids, ...props }) => {
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
        {mentions !== undefined ? markMentions(children) : children}
      </ReactMarkdown>
    </div>
  );
}
