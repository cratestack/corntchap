import { useCallback, type Key, type ReactNode } from "react";
import { type ListSource } from "@/lib/listbox";
import { Picker, type PickerRowState } from "./virtual-combobox";

export interface VirtualMenuProps<T> {
  /** The element that opens the menu: a button (it takes the ref and the popup attributes). */
  trigger: ReactNode;
  /** The menu's accessible name (and the sheet's title on phones). */
  label: string;
  source: ListSource<T>;
  /** The search text changed: read the source again with it. Without it the rows are filtered here by `itemText`. */
  onQuery?: ((q: string) => void) | undefined;
  /** Client-side search text of an option, used when there is no `onQuery`. */
  itemText?: ((item: T) => string) | undefined;
  itemKey: (item: T, index: number) => Key;
  renderItem: (item: T, state: PickerRowState) => ReactNode;
  /**
   * `"action"`: a pick runs `onSelect` and closes; `"single"`: one option is selected (`selected`), a pick closes;
   * `"multi"`: several are (`selected`), a pick toggles and the menu stays open.
   */
  mode: "action" | "single" | "multi";
  /** Which options are selected, in `single` and `multi` mode. */
  selected?: ((item: T) => boolean) | undefined;
  onSelect: (item: T) => void;
  /** Above the search box, e.g. a title. */
  header?: ReactNode;
  /** Whether there is a search box (default true). */
  searchable?: boolean | undefined;
  isDisabled?: ((item: T) => boolean) | undefined;
  empty?: ReactNode;
  defaultOpen?: boolean | undefined;
  align?: "start" | "center" | "end" | undefined;
  className?: string | undefined;
}

/**
 * A menu over a data-driven list that may be thousands long: virtualized, paged, searchable. It replaces
 * `DropdownMenu` where the items come from the server. Options are `role="option"` in a listbox (`aria-selected`),
 * not `menuitem`s, because the list is searched and selected like a combobox's.
 */
export function VirtualMenu<T>({ trigger, label, source, onQuery, itemText, itemKey, renderItem, mode, selected, onSelect, header, searchable = true, isDisabled, empty, defaultOpen, align, className }: VirtualMenuProps<T>) {
  const filter = useCallback((item: T, q: string) => !itemText || itemText(item).toLowerCase().includes(q.trim().toLowerCase()), [itemText]);
  return (
    <Picker<T>
      trigger={trigger}
      label={label}
      source={source}
      onQuery={onQuery}
      filter={filter}
      searchable={searchable}
      itemKey={itemKey}
      renderItem={renderItem}
      selected={mode === "action" ? undefined : selected}
      multiple={mode === "multi"}
      onPick={onSelect}
      isDisabled={isDisabled}
      header={header}
      empty={empty}
      defaultOpen={defaultOpen}
      align={align}
      className={className}
    />
  );
}
