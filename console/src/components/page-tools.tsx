import { PaletteButton } from "./shell";
import { RefreshButton } from "./refresh-button";

/** The palette and refresh buttons every page header carries (`PageShell` has them built in; a `ListPage` takes them as actions). */
export function PageTools() {
  return (
    <>
      <PaletteButton />
      <RefreshButton />
    </>
  );
}
