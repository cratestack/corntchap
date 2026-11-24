import { useState } from "react";
import { Loader2, Pencil } from "lucide-react";
import { toast } from "sonner";
import type { ProjectSummary } from "@corntchap/api";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { errorMessage } from "@/lib/api";
import { useProcCall } from "@/lib/data";
import { Markdown } from "./markdown";
import { Group } from "./bits";

/** The longest guide the server keeps. */
export const GUIDE_MAX = 20000;

const PLACEHOLDER = `# How we work here

- Start with what_now: it says what is ready for your folder and elsewhere.
- Small pull requests, one task each. Name the task id in the title.
- Ask on the pass before touching shared files.`;

/**
 * The pot's guide: how we work here, written by people for agents. Everyone in the pot reads it
 * (the agents get it from `what_now`); owners of the pot's team and admins edit it, as markdown
 * with a preview.
 */
export function PotGuide({ project, canManage }: { project: ProjectSummary; canManage: boolean }) {
  const call = useProcCall();
  const guide = project.guide ?? "";
  const [draft, setDraft] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const editing = draft !== null;

  async function save() {
    if (draft === null) return;
    setBusy(true);
    try {
      await call("setPotGuide", { project: project.key, guide: draft });
      toast.success(draft.trim() ? "Guide saved" : "Guide removed", {
        description: draft.trim() ? "Agents read it with what_now." : undefined,
      });
      setDraft(null);
    } catch (e) {
      toast.error("Could not save the guide", { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Group
      title="Guide for agents"
      aside={
        canManage && !editing ? (
          <Button variant="ghost" size="sm" className="-my-1 h-8 gap-1.5 px-2 text-[12px] pointer-coarse:h-11" onClick={() => setDraft(guide)}>
            <Pencil className="size-3.5" aria-hidden="true" /> {guide ? "Edit" : "Write one"}
          </Button>
        ) : undefined
      }
      hint={
        editing
          ? `How we work here, in markdown. Agents read it with what_now, and its first 1 500 characters come with register_station and whoami. ${draft.length} of ${GUIDE_MAX} characters.`
          : "How we work here, written by people for agents. They read it with what_now."
      }
    >
      <div className="p-3.5">
        {editing ? (
          <form
            className="space-y-3"
            onSubmit={(e) => {
              e.preventDefault();
              void save();
            }}
          >
            <Tabs defaultValue="write">
              <TabsList>
                <TabsTrigger value="write" className="pointer-coarse:min-h-11">
                  Write
                </TabsTrigger>
                <TabsTrigger value="preview" className="pointer-coarse:min-h-11">
                  Preview
                </TabsTrigger>
              </TabsList>
              <TabsContent value="write">
                <Textarea
                  aria-label="Guide, markdown"
                  rows={14}
                  maxLength={GUIDE_MAX}
                  value={draft}
                  onChange={(e) => setDraft(e.target.value)}
                  placeholder={PLACEHOLDER}
                  className="font-mono text-[13px]"
                  autoFocus
                />
              </TabsContent>
              <TabsContent value="preview">
                <div className="min-h-40 rounded-lg border border-border bg-background/60 p-3">
                  {draft.trim() ? <Markdown>{draft}</Markdown> : <p className="text-[13px] text-muted-foreground">Nothing to preview yet.</p>}
                </div>
              </TabsContent>
            </Tabs>
            <div className="flex flex-wrap justify-end gap-2">
              <Button type="button" variant="outline" className="max-sm:h-11 max-sm:flex-1" disabled={busy} onClick={() => setDraft(null)}>
                Cancel
              </Button>
              <Button type="submit" className="max-sm:h-11 max-sm:flex-1" disabled={busy || draft.trim() === guide.trim()}>
                {busy && <Loader2 className="animate-spin" aria-hidden="true" />} Save the guide
              </Button>
            </div>
          </form>
        ) : guide ? (
          <Markdown>{guide}</Markdown>
        ) : (
          <p className="text-[13px] text-muted-foreground">
            {canManage
              ? "No guide yet. Write what an agent should know to do a good job here: conventions, what to do first, where the work is, what needs no checkout."
              : "No guide yet. Owners of the pot's team write it."}
          </p>
        )}
      </div>
    </Group>
  );
}
