import { useEffect, useMemo, useRef, useState, type DragEvent, type ReactNode } from "react";
import { useSearchParams } from "react-router";
import type { FileRef } from "@corntchap/api";
import { Camera, FileIcon, Loader2, Paperclip, Search, Trash2, Upload as UploadIcon, X } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { errorMessage } from "@/lib/api";
import { useInvalidateAll, usePotFiles, useV1Call } from "@/lib/data";
import { dismissUpload, fileHref, fmtBytes, isImage, onUploadsSettled, uploadFiles, useUploads } from "@/lib/files";
import { fmtAgo, plural, useNow } from "@/lib/format";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { EmptyState, RowSkeleton, SectionLabel } from "./bits";
import { RefreshButton } from "./refresh-button";

type Target = { pot: string; task?: string; question?: string };
type AnyFile = Pick<FileRef, "id" | "name" | "content_type" | "size" | "status">;

// ---------------------------------------------------------------- upload tray

/** Progress for uploads running in the background, bottom-left (toasts sit bottom-right). Mounted once in the shell. */
export function UploadTray() {
  const uploads = useUploads();
  const invalidate = useInvalidateAll();
  useEffect(() => onUploadsSettled(() => void invalidate()), [invalidate]);
  if (uploads.length === 0) return null;
  return (
    <div
      role="region"
      aria-label="Uploads"
      className="fixed bottom-[calc(4.5rem+env(safe-area-inset-bottom))] left-4 z-50 flex w-[min(22rem,calc(100vw-2rem))] flex-col gap-2 md:bottom-4 md:left-[88px] lg:left-[272px]"
    >
      {uploads.map((u) => {
        const pct = u.size > 0 ? Math.min(100, Math.round((u.loaded / u.size) * 100)) : 100;
        return (
          <div key={u.id} className="rounded-xl border border-border bg-card px-3 py-2.5 shadow-lg">
            <div className="flex items-center gap-2">
              {u.state === "uploading" ? (
                <Loader2 className="size-3.5 shrink-0 animate-spin text-primary" aria-hidden="true" />
              ) : (
                <Paperclip className={cn("size-3.5 shrink-0", u.state === "error" ? "text-destructive" : "text-success")} aria-hidden="true" />
              )}
              <span className="min-w-0 flex-1 truncate text-[12.5px] font-medium" title={u.name}>
                {u.name}
              </span>
              <span className="shrink-0 font-mono text-[10.5px] text-muted-foreground">
                {u.state === "uploading" ? `${pct}%` : u.state === "done" ? "done" : "failed"}
              </span>
              <button
                type="button"
                onClick={() => dismissUpload(u.id)}
                aria-label={u.state === "uploading" ? `Cancel upload of ${u.name}` : `Dismiss ${u.name}`}
                className="grid size-5 shrink-0 place-items-center rounded text-muted-foreground hover:bg-muted hover:text-foreground"
              >
                <X className="size-3.5" aria-hidden="true" />
              </button>
            </div>
            {u.state === "error" ? (
              <p className="mt-1 truncate text-[11.5px] text-destructive" title={u.error}>
                {u.error}
              </p>
            ) : (
              <div
                role="progressbar"
                aria-valuenow={pct}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-label={`Uploading ${u.name}`}
                className="mt-2 h-1.5 overflow-hidden rounded-full bg-muted"
              >
                <div className={cn("h-full rounded-full transition-[width]", u.state === "done" ? "bg-success" : "bg-primary")} style={{ width: `${pct}%` }} />
              </div>
            )}
            {u.state === "uploading" && (
              <p className="mt-1 font-mono text-[10.5px] text-muted-foreground">
                {fmtBytes(u.loaded)} of {fmtBytes(u.size)}
              </p>
            )}
          </div>
        );
      })}
    </div>
  );
}

// ---------------------------------------------------------------- drop zone + attach button

/** Wraps content so files dropped on it upload (attached to the task/question, or loose in the pot). */
export function FileDrop({ target, children, className, label = "Drop to attach" }: { target: Target; children: ReactNode; className?: string; label?: string }) {
  const [over, setOver] = useState(false);
  const depth = useRef(0);
  const hasFiles = (e: DragEvent) => Array.from(e.dataTransfer.types).includes("Files");
  return (
    <div
      className={cn("relative", className)}
      onDragEnter={(e) => {
        if (!hasFiles(e)) return;
        e.preventDefault();
        depth.current++;
        setOver(true);
      }}
      onDragOver={(e) => {
        if (hasFiles(e)) e.preventDefault();
      }}
      onDragLeave={(e) => {
        if (!hasFiles(e)) return;
        depth.current = Math.max(0, depth.current - 1);
        if (depth.current === 0) setOver(false);
      }}
      onDrop={(e) => {
        if (!hasFiles(e)) return;
        e.preventDefault();
        depth.current = 0;
        setOver(false);
        uploadFiles(Array.from(e.dataTransfer.files), target);
      }}
    >
      {children}
      {over && (
        <div className="pointer-events-none absolute inset-0 z-10 grid place-items-center rounded-xl border-2 border-dashed border-primary bg-primary/10 text-[13px] font-semibold text-primary backdrop-blur-[1px]">
          {label}
        </div>
      )}
    </div>
  );
}

export function AttachButton({ target, children, size = "sm", variant = "outline" }: { target: Target; children?: ReactNode; size?: "sm" | "default"; variant?: "outline" | "ghost" | "default" }) {
  const input = useRef<HTMLInputElement>(null);
  return (
    <>
      <input
        ref={input}
        type="file"
        multiple
        hidden
        tabIndex={-1}
        onChange={(e) => {
          uploadFiles(Array.from(e.target.files ?? []), target);
          e.target.value = "";
        }}
      />
      <Button type="button" size={size} variant={variant} onClick={() => input.current?.click()}>
        <Paperclip /> {children ?? "Attach"}
      </Button>
    </>
  );
}

// ---------------------------------------------------------------- delete

function useDeleteFile() {
  const call = useV1Call();
  const [file, setFile] = useState<AnyFile | null>(null);
  const [busy, setBusy] = useState(false);
  const dialog = (
    <AlertDialog open={!!file} onOpenChange={(o) => !o && !busy && setFile(null)}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete {file?.name}?</AlertDialogTitle>
          <AlertDialogDescription className="break-words">
            {file?.id} is removed from storage and every link to it stops working. This cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Keep it</AlertDialogCancel>
          <AlertDialogAction
            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            onClick={(e) => {
              e.preventDefault();
              if (!file) return;
              setBusy(true);
              call("DELETE", `/files/${encodeURIComponent(file.id)}`)
                .then(() => {
                  toast.success(`${file.name} deleted`);
                  setFile(null);
                })
                .catch((err: unknown) => toast.error("Could not delete the file", { description: errorMessage(err) }))
                .finally(() => setBusy(false));
            }}
          >
            {busy && <Loader2 className="animate-spin" />} Delete
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
  return { ask: setFile, dialog };
}

// ---------------------------------------------------------------- shared row bits

const STATUS_TONE: Record<string, string> = {
  uploading: "border-corn/50 bg-corn/15 text-corn-text",
  failed: "border-destructive/40 bg-destructive/10 text-destructive",
  deleted: "border-border bg-muted text-muted-foreground",
};

function StatusBadge({ status }: { status: string }) {
  if (status === "ready") return null;
  return (
    <span className={cn("shrink-0 rounded-full border px-1.5 py-px font-mono text-[10px] uppercase tracking-wide", STATUS_TONE[status] ?? STATUS_TONE.deleted)}>
      {status}
    </span>
  );
}

function Thumb({ f, className }: { f: AnyFile; className?: string }) {
  const [broken, setBroken] = useState(false);
  if (isImage(f.content_type) && f.status === "ready" && !broken) {
    return (
      <img
        src={fileHref(f.id, f.name)}
        alt=""
        loading="lazy"
        onError={() => setBroken(true)}
        className={cn("shrink-0 rounded-md border border-border bg-muted object-cover", className)}
      />
    );
  }
  return (
    <span className={cn("grid shrink-0 place-items-center rounded-md border border-border bg-muted text-muted-foreground", className)}>
      <FileIcon className="size-4" aria-hidden="true" />
    </span>
  );
}

function FileRow({ f, compact, onDelete, extra }: { f: AnyFile; compact?: boolean; onDelete: (f: AnyFile) => void; extra?: ReactNode }) {
  const linkable = f.status === "ready";
  const name = (
    <span className="block truncate text-[13px] font-medium" title={f.name}>
      {f.name}
    </span>
  );
  return (
    <li className={cn("flex min-w-0 items-center gap-2.5 rounded-lg border border-border bg-card", compact ? "px-2 py-1.5" : "px-3 py-2 pointer-coarse:min-h-16")}>
      <Thumb f={f} className={compact ? "size-8" : "size-10 pointer-coarse:size-11"} />
      <div className="min-w-0 flex-1">
        {linkable ? (
          <a href={fileHref(f.id, f.name)} target="_blank" rel="noreferrer" className="block min-w-0 rounded underline-offset-2 hover:underline focus-visible:underline">
            {name}
          </a>
        ) : (
          name
        )}
        <p className="truncate font-mono text-[10.5px] text-muted-foreground">
          {f.id}
          {f.size != null && ` · ${fmtBytes(f.size)}`}
          {extra}
        </p>
      </div>
      <StatusBadge status={f.status} />
      <button
        type="button"
        onClick={() => onDelete(f)}
        aria-label={`Delete ${f.name}`}
        className="grid size-8 shrink-0 place-items-center rounded-lg text-muted-foreground hover:bg-destructive/10 hover:text-destructive pointer-coarse:size-11"
      >
        <Trash2 className="size-3.5" aria-hidden="true" />
      </button>
    </li>
  );
}

// ---------------------------------------------------------------- attachments (task sheet, question cards)

/** A task's or question's files with a drop zone and an attach button. */
export function Attachments({ target, files, compact = false }: { target: Target; files: FileRef[]; compact?: boolean }) {
  const del = useDeleteFile();
  const owner = target.task ?? target.question ?? "";
  return (
    <FileDrop target={target} label={`Drop to attach to ${owner}`}>
      <section aria-label="Attachments" className={cn("rounded-xl", compact ? "space-y-1.5" : "space-y-2.5")}>
        <div className="flex items-center justify-between gap-2">
          <SectionLabel>Attachments{files.length > 0 && ` · ${files.length}`}</SectionLabel>
          <AttachButton target={target} variant={compact ? "ghost" : "outline"} />
        </div>
        {files.length === 0 ? (
          <p className={cn("text-muted-foreground", compact ? "text-[12px]" : "text-[13px]")}>
            {compact ? "Drop files here or attach one." : "No files yet. Drop them here or use Attach."}
          </p>
        ) : (
          <ul className="space-y-1.5">
            {files.map((f) => (
              <FileRow key={f.id} f={f} compact={compact} onDelete={del.ask} />
            ))}
          </ul>
        )}
      </section>
      {del.dialog}
    </FileDrop>
  );
}

// ---------------------------------------------------------------- the pot's files view

export function FilesView({ potKey }: { potKey: string }) {
  const [params, setParams] = useSearchParams();
  const q = params.get("q") ?? "";
  const [text, setText] = useState(q);
  useEffect(() => setText(q), [q]);
  useEffect(() => {
    if (text === q) return;
    const t = setTimeout(
      () =>
        setParams(
          (p) => {
            const n = new URLSearchParams(p);
            if (text.trim()) n.set("q", text);
            else n.delete("q");
            return n;
          },
          { replace: true },
        ),
      250,
    );
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [text]);

  const res = usePotFiles(potKey);
  const del = useDeleteFile();
  const now = useNow();
  const all = useMemo(() => (res.data ?? []).filter((f) => f.status !== "deleted"), [res.data]);
  const rows = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return needle ? all.filter((f) => f.name.toLowerCase().includes(needle) || f.id.toLowerCase().includes(needle)) : all;
  }, [all, q]);
  const input = useRef<HTMLInputElement>(null);
  const camera = useRef<HTMLInputElement>(null);
  const target = { pot: potKey };

  return (
    <FileDrop target={target} label="Drop to upload to this pot" className="mx-auto max-w-3xl">
      <div className="flex flex-wrap items-center gap-2" role="search" aria-label="Search files">
        <div className="relative min-w-0 flex-1 basis-full sm:basis-60">
          <Search className="pointer-events-none absolute top-1/2 left-3 size-3.5 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
          <Input
            type="search"
            aria-label="Filter files by name"
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder="Filter by name…"
            className="h-11 rounded-full pr-3 pl-9 text-[14px] md:h-9 md:text-[13px]"
          />
        </div>
        <RefreshButton />
        <AttachButton target={target} size="default" variant="default">
          Upload
        </AttachButton>
      </div>

      <button
        type="button"
        onClick={() => input.current?.click()}
        className="mt-4 flex min-h-14 w-full items-center justify-center max-md:hidden gap-2 rounded-xl border-2 border-dashed border-border px-4 py-4 text-[13px] text-muted-foreground transition-colors hover:border-primary/50 hover:text-foreground"
      >
        <UploadIcon className="size-4" aria-hidden="true" />
        Drop files anywhere on this page, or click to choose
      </button>
      <Button type="button" variant="outline" className="mt-2 w-full md:hidden" onClick={() => camera.current?.click()}>
        <Camera /> Take a photo
      </Button>
      <input
        ref={camera}
        type="file"
        accept="image/*"
        capture="environment"
        hidden
        tabIndex={-1}
        onChange={(e) => {
          uploadFiles(Array.from(e.target.files ?? []), target);
          e.target.value = "";
        }}
      />
      <input
        ref={input}
        type="file"
        multiple
        hidden
        tabIndex={-1}
        onChange={(e) => {
          uploadFiles(Array.from(e.target.files ?? []), target);
          e.target.value = "";
        }}
      />

      <div className="mt-5">
        {res.isLoading ? (
          <RowSkeleton rows={4} />
        ) : res.error && !res.data ? (
          <EmptyState band={false} title="The files would not load">
            {errorMessage(res.error)}
          </EmptyState>
        ) : all.length === 0 ? (
          <EmptyState title="No files in this pot yet">Upload screenshots, logs or build output. Cooks can attach files too.</EmptyState>
        ) : rows.length === 0 ? (
          <EmptyState band={false} title="No file by that name">
            {plural(all.length, "file")} in the pot, none matching “{q}”.
          </EmptyState>
        ) : (
          <ul className="space-y-1.5">
            {rows.map((f) => (
              <FileRow
                key={f.id}
                f={f}
                onDelete={del.ask}
                extra={
                  <>
                    {f.task && ` · ${f.task}`}
                    {f.question && ` · ${f.question}`}
                    {f.created_by && ` · ${f.created_by}`}
                    {` · ${fmtAgo(f.created_at, now)}`}
                    {f.status === "failed" && f.error && ` · ${f.error}`}
                  </>
                }
              />
            ))}
          </ul>
        )}
      </div>
      {del.dialog}
    </FileDrop>
  );
}
