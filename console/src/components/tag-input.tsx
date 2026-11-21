import { useId, useState, type KeyboardEvent } from "react";
import { X } from "lucide-react";
import { cn } from "@/lib/utils";

/** Chips input: Enter, comma or Tab commits the text; Backspace on an empty field removes the last chip. */
export function TagInput({
  value,
  onChange,
  placeholder,
  suggestions,
  normalize = (s) => s,
  label,
  className,
}: {
  value: string[];
  onChange: (v: string[]) => void;
  placeholder?: string;
  suggestions?: string[];
  normalize?: (s: string) => string;
  label: string;
  className?: string;
}) {
  const [text, setText] = useState("");
  const listId = useId();

  const commit = (raw: string) => {
    const parts = raw
      .split(",")
      .map((s) => normalize(s.trim()))
      .filter(Boolean);
    if (parts.length === 0) return;
    const next = [...value];
    for (const p of parts) if (!next.includes(p)) next.push(p);
    onChange(next);
    setText("");
  };

  const onKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" || e.key === ",") {
      if (text.trim()) {
        e.preventDefault();
        commit(text);
      }
    } else if (e.key === "Backspace" && !text && value.length) {
      onChange(value.slice(0, -1));
    }
  };

  return (
    <div
      className={cn(
        "flex min-h-9 flex-wrap items-center gap-1.5 rounded-md border border-input bg-card px-2 py-1 shadow-xs focus-within:ring-[3px] focus-within:ring-ring/50",
        className,
      )}
    >
      {value.map((v) => (
        <span key={v} className="inline-flex items-center gap-1 rounded-full bg-muted px-2 py-0.5 text-[12px] font-medium">
          {v}
          <button
            type="button"
            aria-label={`Remove ${v}`}
            className="rounded-full text-muted-foreground hover:text-foreground"
            onClick={() => onChange(value.filter((x) => x !== v))}
          >
            <X className="size-3" />
          </button>
        </span>
      ))}
      <input
        aria-label={label}
        value={text}
        list={suggestions ? listId : undefined}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKey}
        onBlur={() => commit(text)}
        placeholder={value.length ? "" : placeholder}
        className="min-w-24 flex-1 bg-transparent py-0.5 text-sm outline-none placeholder:text-muted-foreground"
      />
      {suggestions && (
        <datalist id={listId}>
          {suggestions.filter((s) => !value.includes(s)).map((s) => (
            <option key={s} value={s} />
          ))}
        </datalist>
      )}
    </div>
  );
}
