import { useSyncExternalStore } from "react";
import { toast } from "sonner";
import type { FileView } from "@corntchap/api";
import { CONSOLE_HEADER, ApiError, redirectToLogin } from "./api";

/** The permanent, login-protected link to a file (302s to a presigned download). */
export const fileHref = (id: string, name: string) => `/files/${encodeURIComponent(id)}/${encodeURIComponent(name)}`;

export const isImage = (contentType: string) => /^image\/(png|jpe?g|gif|webp|avif|svg\+xml)$/i.test(contentType);

export function fmtBytes(n: number | null | undefined): string {
  if (n == null) return "";
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${units[i]}`;
}

export const MAX_FILE_BYTES = 1024 ** 3;

// ---------------------------------------------------------------- background upload queue

export interface Upload {
  id: number;
  name: string;
  pot: string;
  size: number;
  loaded: number;
  state: "uploading" | "done" | "error";
  error?: string;
}

interface Target {
  pot: string;
  task?: string | undefined;
  question?: string | undefined;
}

let seq = 0;
let uploads: Upload[] = [];
const aborts = new Map<number, () => void>();
const listeners = new Set<() => void>();
const doneHooks = new Set<() => void>();

function set(next: Upload[]) {
  uploads = next;
  listeners.forEach((l) => l());
}
function patch(id: number, p: Partial<Upload>) {
  set(uploads.map((u) => (u.id === id ? { ...u, ...p } : u)));
}

/** Called by the tray (mounted once) so finished uploads refresh the queries on screen. */
export function onUploadsSettled(fn: () => void): () => void {
  doneHooks.add(fn);
  return () => doneHooks.delete(fn);
}

export function useUploads(): Upload[] {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => uploads,
  );
}

export function dismissUpload(id: number) {
  aborts.get(id)?.();
  aborts.delete(id);
  set(uploads.filter((u) => u.id !== id));
}

/** Uploads continue in the background (module state), so closing a sheet or changing page never stops them. */
export function uploadFiles(files: Iterable<File>, t: Target) {
  for (const file of files) {
    if (file.size > MAX_FILE_BYTES) {
      toast.error(`${file.name} is too big`, { description: "Files can be up to 1 GiB." });
      continue;
    }
    startUpload(file, t);
  }
}

function startUpload(file: File, t: Target) {
  const id = ++seq;
  set([...uploads, { id, name: file.name, pot: t.pot, size: file.size, loaded: 0, state: "uploading" }]);
  const qs = new URLSearchParams({ name: file.name });
  if (t.task) qs.set("task", t.task);
  if (t.question) qs.set("question", t.question);
  const xhr = new XMLHttpRequest();
  aborts.set(id, () => xhr.abort());
  xhr.open("POST", `/api/v1/projects/${encodeURIComponent(t.pot)}/files?${qs}`);
  xhr.withCredentials = true;
  for (const [k, v] of Object.entries(CONSOLE_HEADER)) xhr.setRequestHeader(k, v);
  xhr.setRequestHeader("content-type", file.type || "application/octet-stream");
  xhr.upload.onprogress = (e) => {
    if (e.lengthComputable) patch(id, { loaded: e.loaded });
  };
  const fail = (message: string) => {
    aborts.delete(id);
    patch(id, { state: "error", error: message });
    toast.error(`Could not upload ${file.name}`, { description: message });
  };
  xhr.onerror = () => fail("Network error");
  xhr.onload = () => {
    if (xhr.status === 401) {
      redirectToLogin();
      return;
    }
    if (xhr.status < 200 || xhr.status >= 300) {
      let message = `Upload failed (${xhr.status})`;
      try {
        const j = JSON.parse(xhr.responseText) as { message?: string };
        if (j.message) message = j.message;
      } catch {
        /* not json */
      }
      fail(new ApiError(xhr.status, "error", message).message);
      return;
    }
    aborts.delete(id);
    patch(id, { state: "done", loaded: file.size });
    toast.success(`${file.name} uploaded`);
    doneHooks.forEach((h) => h());
    // Finished rows linger briefly so a fast upload still reads as having happened.
    setTimeout(() => set(uploads.filter((u) => u.id !== id)), 2500);
  };
  xhr.send(file);
}

export type { FileView };
