import { ExternalLink } from "lucide-react";

/** A station's name, as a link to its session page (a cloud run's URL) when it has one. */
export function StationLink({ label, url, title, className }: { label: string; url?: string | null | undefined; title?: string | undefined; className?: string | undefined }) {
  if (!url) {
    return (
      <span className={className} title={title}>
        {label}
      </span>
    );
  }
  return (
    <a
      href={url}
      target="_blank"
      rel="noreferrer"
      title={title ?? url}
      className={`${className ?? ""} inline-flex max-w-full items-center gap-0.5 underline-offset-2 hover:text-foreground hover:underline focus-visible:underline`}
    >
      <span className="truncate">{label}</span>
      <ExternalLink className="size-3 shrink-0" aria-hidden="true" />
    </a>
  );
}

/** A short label for a session URL: its last path segment, or the host. */
export function shortUrl(url: string): string {
  try {
    const u = new URL(url);
    const seg = u.pathname.split("/").filter(Boolean).pop();
    return seg ? (seg.length > 18 ? `${seg.slice(0, 16)}…` : seg) : u.host;
  } catch {
    return "session";
  }
}
