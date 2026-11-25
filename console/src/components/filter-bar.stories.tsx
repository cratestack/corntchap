import { useMemo } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Tag, UserRound } from "lucide-react";
import { staticSource } from "@/lib/listbox";
import { useUrlFilters, type UrlFilterSchema } from "@/lib/url-filters";
import { FilterBar, type FacetDef, type FacetOption } from "./filter-bar";
import { errorSource, loadingSource, makeLabels, makePeople, matchString, usePagedSource } from "./ui/virtual.fixtures";

const owners: FacetOption[] = makePeople(10_000).map((p) => ({ value: p.name, label: p.name, hint: p.kind }));
const labels: FacetOption[] = makeLabels(10_000).map((l) => ({ value: l, label: l }));
const matchOption = (o: FacetOption, q: string) => matchString(`${o.label} ${o.hint ?? ""}`, q);

const meta: Meta = {
  title: "Console/FilterBar",
  decorators: [(Story) => <div className="min-h-dvh bg-background p-4 md:p-8"><Story /></div>],
};
export default meta;
type Story = StoryObj;

const SCHEMA = { owner: { type: "string" }, label: { type: "list" } } as const;

function Bar({ owner, label }: { owner: FacetDef["source"]; label: FacetDef["source"]; ownerQuery?: (q: string) => void; labelQuery?: (q: string) => void }) {
  const f = useUrlFilters(SCHEMA);
  const facets: FacetDef[] = [
    { id: "owner", label: "Owner", icon: <UserRound className="size-3 text-primary" aria-hidden="true" />, source: owner },
    { id: "label", label: "Label", icon: <Tag className="size-3 text-primary" aria-hidden="true" />, source: label, multiple: true },
  ];
  return (
    <div className="space-y-4">
      <FilterBar q={f.q} onQ={f.setQ} facets={facets} values={{ owner: f.values.owner, label: f.values.label }} onChange={(p) => f.set(p as never)} onClear={f.clear} searchPlaceholder="Search tasks" />
      <p className="font-mono text-[12px] text-muted-foreground">
        q={JSON.stringify(f.q)} owner={JSON.stringify(f.values.owner)} label={JSON.stringify(f.values.label)}
      </p>
    </div>
  );
}

/** A search box and chips whose menus list 10,000 options each (searched here). Owner is one value, Label several. */
export const TenThousand: Story = {
  render: function Render() {
    const o = useMemo(() => staticSource(owners), []);
    const l = useMemo(() => staticSource(labels), []);
    return <Bar owner={o} label={l} />;
  },
};

export const Phone: Story = { ...TenThousand, globals: { viewport: { value: "phone", isRotated: false } } };

/** Owner options are paged and searched by the server. */
export const Paged: Story = {
  render: function Render() {
    const o = usePagedSource(owners, { match: matchOption });
    const l = useMemo(() => staticSource(labels), []);
    return <Bar owner={o.source} label={l} />;
  },
};

/** A chip whose value is not among the options loaded (it came back from the URL) still names it, through `valueText`. */
export const RestoredValues: Story = {
  render: function Render() {
    const named = useMemo(() => makePeople(10_000).map((p) => ({ value: p.name, label: p.display, hint: p.kind })), []);
    const o = usePagedSource(named, { match: matchOption });
    const facets: FacetDef[] = [{ id: "owner", label: "Owner", source: o.source, onQuery: o.onQuery, valueText: (v) => `Cook number ${v.slice(5)}` }];
    return <FilterBar q="" onQ={() => {}} facets={facets} values={{ owner: "cook-9000" }} onChange={() => {}} onClear={() => {}} />;
  },
};

const DEFAULTED = { owner: { type: "string", default: "cook-1" }, label: { type: "list", default: ["label-1"] }, kind: { type: "string" } } satisfies UrlFilterSchema;

/**
 * Filters with non-empty defaults. "No owner" is kept (`?owner=`) instead of falling back to the default, and
 * "Owner none, kind task, search x" changes three things in one click: none of them is lost.
 */
export const Defaults: Story = {
  render: function Render() {
    const f = useUrlFilters(DEFAULTED);
    return (
      <div className="space-y-3">
        <div className="flex flex-wrap gap-2">
          <button type="button" onClick={() => f.set({ owner: "", label: [] })} className="h-8 rounded-full border border-input px-3 text-[12.5px]">
            No owner, no labels
          </button>
          <button
            type="button"
            onClick={() => {
              f.set({ owner: "" });
              f.set({ kind: "task" });
              f.setQ("x");
            }}
            className="h-8 rounded-full border border-input px-3 text-[12.5px]"
          >
            Owner none, kind task, search x
          </button>
          <button type="button" onClick={f.clear} className="h-8 rounded-full border border-input px-3 text-[12.5px]">
            Back to the defaults
          </button>
        </div>
        <p data-testid="url-state" className="font-mono text-[12px] text-muted-foreground">
          q={JSON.stringify(f.q)} owner={JSON.stringify(f.values.owner)} label={JSON.stringify(f.values.label)} kind={JSON.stringify(f.values.kind)}
        </p>
      </div>
    );
  },
};

export const Empty: Story = {
  render: function Render() {
    const none = useMemo(() => staticSource<FacetOption>([]), []);
    return <Bar owner={none} label={none} />;
  },
};

export const Loading: Story = { render: () => <Bar owner={loadingSource()} label={loadingSource()} /> };

export const Error: Story = { render: () => <Bar owner={errorSource()} label={errorSource()} /> };
