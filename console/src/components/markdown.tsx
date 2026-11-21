import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { cn } from "@/lib/utils";

/** Markdown for task bodies and comments. Raw HTML is never rendered; links open in a new tab. */
export function Markdown({ children, className }: { children: string; className?: string }) {
  return (
    <div className={cn("prose-ct", className)}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          a: ({ node: _n, ...props }) => <a {...props} target="_blank" rel="noreferrer noopener" />,
        }}
      >
        {children}
      </ReactMarkdown>
    </div>
  );
}
