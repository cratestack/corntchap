import { useState, type FormEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { errorMessage } from "@/lib/api";
import { useV1Call } from "@/lib/data";

/** Change your own handle, the @name others mention. The server decides what is valid and taken (400 / 409). */
export function HandleDialog({ open, onOpenChange, current }: { open: boolean; onOpenChange: (o: boolean) => void; current: string }) {
  const call = useV1Call();
  const queryClient = useQueryClient();
  const [handle, setHandle] = useState(current);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function save(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const out = await call<{ name: string }>("PATCH", "/me", { name: handle });
      await queryClient.invalidateQueries({ queryKey: ["session"] });
      toast.success(`You are now @${out.name}`);
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
        if (o) {
          setHandle(current);
          setError(null);
        }
        onOpenChange(o);
      }}
    >
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Change your handle</DialogTitle>
          <DialogDescription>
            The name others use to @mention you. Letters, digits, dots, dashes and underscores; teams and tasks stay with you.
          </DialogDescription>
        </DialogHeader>
        <form className="space-y-4" onSubmit={(e) => void save(e)}>
          <div className="space-y-1.5">
            <Label htmlFor="handle">Handle</Label>
            <div className="flex items-center gap-1.5">
              <span className="font-mono text-muted-foreground" aria-hidden="true">
                @
              </span>
              <Input
                id="handle"
                required
                autoFocus
                maxLength={64}
                value={handle}
                onChange={(e) => setHandle(e.target.value)}
                className="font-mono"
                aria-invalid={!!error}
                aria-describedby={error ? "handle-error" : undefined}
              />
            </div>
            {error && (
              <p id="handle-error" role="alert" className="text-[12.5px] text-destructive">
                {error}
              </p>
            )}
          </div>
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy || !handle.trim() || handle.trim().toLowerCase() === current}>
              {busy && <Loader2 className="animate-spin" />} Save
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
