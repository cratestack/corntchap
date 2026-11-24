import { useMemo } from "react";
import { Select, SelectContent, SelectGroup, SelectItem, SelectLabel, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useProjects, useTeamChefs, type MemberRow } from "@/lib/data";
import { cn } from "@/lib/utils";
import { Avatar } from "./bits";

const NONE = "-";

/** The pot's team chefs for an owner picker: people first, then cooks (each by handle). */
export function useOwnerChoices(pot: string, enabled = true): { people: MemberRow[]; cooks: MemberRow[]; loading: boolean } {
  const team = useProjects().data?.find((p) => p.key === pot)?.team;
  const chefs = useTeamChefs(team, { enabled });
  return useMemo(() => {
    const all = chefs.data ?? [];
    return { people: all.filter((m) => m.kind === "user"), cooks: all.filter((m) => m.kind !== "user"), loading: chefs.isLoading };
  }, [chefs.data, chefs.isLoading]);
}

function Who({ m }: { m: MemberRow }) {
  return (
    <span className="inline-flex items-center gap-2">
      <Avatar name={m.name} className="size-4 rounded-[5px] text-[9px]" />
      <span>{m.display_name ? `${m.display_name} · @${m.name}` : m.name}</span>
    </span>
  );
}

/**
 * Who owns a task: a picker over the pot's team (people first, then cooks) with "No owner".
 * `value` is a handle or null; `onChange` gets a handle, or "" to clear.
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
  const { people, cooks } = useOwnerChoices(pot);
  // an owner who left the team is still shown, so the field never lies about what is stored
  const known = [...people, ...cooks].some((m) => m.name === value);
  return (
    <Select value={value ?? NONE} onValueChange={(v) => v !== (value ?? NONE) && onChange(v === NONE ? "" : v)} disabled={disabled}>
      <SelectTrigger
        id={id}
        aria-label="Owner"
        size={pill ? "sm" : "default"}
        className={cn(pill ? "h-8 min-w-0 max-w-52 gap-1.5 rounded-full border-border px-3 text-[12.5px] data-[size=sm]:h-8" : "w-full")}
      >
        <SelectValue>
          <span className="inline-flex min-w-0 items-center gap-1.5">
            {pill && <span className="text-muted-foreground">Owner</span>}
            {value ? (
              <>
                <Avatar name={value} className="size-4 rounded-[5px] text-[9px]" />
                <span className="truncate">{value}</span>
              </>
            ) : (
              <span className="text-muted-foreground">{pill ? "none" : "No owner yet"}</span>
            )}
          </span>
        </SelectValue>
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={NONE}>No owner</SelectItem>
        {value && !known && (
          <SelectItem value={value}>
            <span className="inline-flex items-center gap-2">
              <Avatar name={value} className="size-4 rounded-[5px] text-[9px]" /> {value}
            </span>
          </SelectItem>
        )}
        {people.length > 0 && (
          <SelectGroup>
            <SelectLabel>People</SelectLabel>
            {people.map((m) => (
              <SelectItem key={m.name} value={m.name}>
                <Who m={m} />
              </SelectItem>
            ))}
          </SelectGroup>
        )}
        {cooks.length > 0 && (
          <SelectGroup>
            <SelectLabel>Cooks</SelectLabel>
            {cooks.map((m) => (
              <SelectItem key={m.name} value={m.name}>
                <Who m={m} />
              </SelectItem>
            ))}
          </SelectGroup>
        )}
      </SelectContent>
    </Select>
  );
}
