import type { MemberView } from "@corntchap/api";
import { VirtualCombobox } from "@/components/ui/virtual-combobox";
import { useProjects } from "@/lib/data";
import { useTeamMembersSource } from "@/lib/sources";
import { useStoreValue } from "@/lib/use-store-value";
import { Avatar } from "./bits";

const itemKey = (m: MemberView) => m.name;
const itemText = (m: MemberView) => (m.display_name ? `${m.display_name} @${m.name}` : m.name);

function Who({ m }: { m: MemberView }) {
  return (
    <span className="inline-flex min-w-0 items-center gap-2">
      <Avatar name={m.name} className="size-4 rounded-[5px] text-[9px]" />
      <span className="truncate">{m.display_name ? `${m.display_name} · @${m.name}` : m.name}</span>
      <span className="shrink-0 text-[12px] text-muted-foreground">{m.kind === "user" ? "person" : "cook"}</span>
    </span>
  );
}

/**
 * Who owns a task: a searchable picker over the pot's team (paged, searched by the server) with "No owner".
 * `value` is a handle or null; `onChange` gets a handle, or "" to clear. An owner who left the team is still shown.
 */
export function OwnerPicker({
  pot,
  value,
  onChange,
  disabled,
  pill = false,
  id,
}: {
  pot: string;
  value: string | null;
  onChange: (handle: string) => void;
  disabled?: boolean;
  /** The compact round trigger used among the task sheet's properties. */
  pill?: boolean;
  id?: string;
}) {
  const team = useProjects().data?.find((p) => p.key === pot)?.team ?? null;
  const q = useStoreValue("");
  const source = useTeamMembersSource(team ?? "", { q: q.value, enabled: !!team });
  return (
    <VirtualCombobox<MemberView>
      label="Owner"
      placeholder="No owner yet"
      source={source}
      onQuery={q.set}
      itemKey={itemKey}
      itemText={itemText}
      renderItem={(m) => <Who m={m} />}
      nullOption="No owner"
      value={value}
      onChange={(v) => (v ?? "") !== (value ?? "") && onChange(v ?? "")}
      trigger={pill ? "pill" : "field"}
      disabled={disabled || !team}
      id={id}
    />
  );
}
