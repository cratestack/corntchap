import { useState, type FormEvent, type ReactNode } from "react";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { errorMessage } from "@/lib/api";

/**
 * Rename something: one text field, saved through `onSave` (which calls a procedure). The server decides
 * what is valid or taken, and its message is shown under the field.
 */
export function RenameDialog({
  open,
  onOpenChange,
  title,
  description,
  label,
  current,
  mono = true,
  onSave,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  title: string;
  description?: ReactNode;
  label: string;
  current: string;
  mono?: boolean;
  onSave: (value: string) => Promise<void>;
}) {
  const [value, setValue] = useState(current);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // start from the current value each time the dialog opens
  const [wasOpen, setWasOpen] = useState(false);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setValue(current);
      setError(null);
    }
  }

  async function save(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await onSave(value.trim());
      onOpenChange(false);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (!busy) onOpenChange(o);
      }}
    >
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          {description && <DialogDescription>{description}</DialogDescription>}
        </DialogHeader>
        <form className="space-y-4" onSubmit={(e) => void save(e)}>
          <div className="space-y-1.5">
            <Label htmlFor="rename-value">{label}</Label>
            <Input
              id="rename-value"
              required
              autoFocus
              maxLength={120}
              value={value}
              onChange={(e) => setValue(e.target.value)}
              className={mono ? "font-mono" : undefined}
              aria-invalid={!!error}
              aria-describedby={error ? "rename-error" : undefined}
            />
            {error && (
              <p id="rename-error" role="alert" className="text-[12.5px] text-destructive">
                {error}
              </p>
            )}
          </div>
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)} disabled={busy}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy || !value.trim() || value.trim() === current}>
              {busy && <Loader2 className="animate-spin" />} Rename
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/**
 * Delete something for good, asked in an AlertDialog. With `confirmText` the person has to type it
 * (a pot's key, a team's name) before the button wakes up. The server refuses what must not go (409/403)
 * and its message is shown in the dialog.
 */
export function DeleteDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmText,
  action = "Delete",
  onConfirm,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  title: string;
  description: ReactNode;
  confirmText?: string | undefined;
  action?: string;
  onConfirm: () => Promise<void>;
}) {
  const [typed, setTyped] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [wasOpen, setWasOpen] = useState(false);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setTyped("");
      setError(null);
    }
  }
  const ready = !confirmText || typed.trim().toLowerCase() === confirmText.toLowerCase();

  async function go() {
    setBusy(true);
    setError(null);
    try {
      await onConfirm();
      onOpenChange(false);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <AlertDialog
      open={open}
      onOpenChange={(o) => {
        if (!busy) onOpenChange(o);
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{title}</AlertDialogTitle>
          <AlertDialogDescription>{description}</AlertDialogDescription>
        </AlertDialogHeader>
        {confirmText && (
          <div className="space-y-1.5">
            <Label htmlFor="delete-confirm">
              Type <b className="font-mono">{confirmText}</b> to confirm
            </Label>
            <Input
              id="delete-confirm"
              autoFocus
              autoComplete="off"
              value={typed}
              onChange={(e) => setTyped(e.target.value)}
              className="font-mono"
            />
          </div>
        )}
        {error && (
          <p role="alert" className="text-[12.5px] text-destructive">
            {error}
          </p>
        )}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={busy}>Keep it</AlertDialogCancel>
          <AlertDialogAction
            disabled={busy || !ready}
            className="bg-destructive text-white hover:bg-destructive/90"
            onClick={(e) => {
              e.preventDefault();
              void go();
            }}
          >
            {busy && <Loader2 className="animate-spin" />} {action}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
