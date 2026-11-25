# Console design (locked)

Canvas: https://claude.ai/artifact/MbtBR7Pv96TZygu5TQNqnD — row "Final". Stack: Vite + React + Refine (`@cratestack/refine`) + Tailwind v4 + shadcn/ui, embedded in the server binary.

**Decision:** layout from A (grouped task rows with progress, cooks/fire rail, "From the kitchen" activity rail), colors from C (wax), sign-in/device approval from B (split screen), the cooking-pot logo. Projects ("pots") can have several **views**: List (default), Board (kanban), Activity, + custom.

## Vocabulary
Pots = projects · Cooks = agents · Fire = relay · Kitchen = board · Scoop = claim next · Serve = complete.
Statuses: **Ready to scoop** (open+ready) · **On the fire** (claimed) · **Waiting** (blocked) · **Served** (done) · **Tossed** (cancelled by a human, with an optional reason; can be restored).

## Tokens (shadcn / Tailwind v4 `@theme`)
| token | light | dark | role |
|---|---|---|---|
| `--background` | `#fbf5e9` cassava cream | `#0f1129` | canvas |
| `--card` | `#fffaf0` | `#161a3d` | rows, cards |
| `--muted` | `#f1e7d3` | `#1d2148` | tabs track, column bg (`#f6ecd9`) |
| `--border` | `#e7dcc6` | `#262a55` | 1px rules |
| `--foreground` | `#16130f` | `#f3ecdd` | text |
| `--muted-foreground` | `#6a5f53` | `#a9acd0` | secondary text (≥4.5:1) |
| `--sidebar` | `#15183a` indigo night (`#1f2350` panels) | `#0a0c1f` | chrome |
| `--primary` | `#c8502c` terracotta | `#e0663f` | actions; light mode: 1.5px ink border + 2px offset ink shadow |
| `--accent` | `#f2b33d` saffron corn | same | Ready + **live** signals (cook awake, lease ticking, fire lit), selected pot |
| `--success` | `#1f7a72` ndolé | `#3fb3a4` | Served |
| `--destructive` | `#9b2f17` | `#f0605a` | toss, errors |
| status Waiting | `#15183a` | `#a9acd0` | |
| `--radius` | 10px rows · 14px columns · 12px big buttons | | |

Lease timer text in light mode uses `#9b5a00` (saffron is too light on cream).

## Type
Bricolage Grotesque 800 (page titles, group/column headings) · Inter 400–600 (UI, 13–14px) · Space Mono (ids `PAY-42`, epochs, counts, timers, section labels in caps).

## Logo
Pot (terracotta body, cream rim line) with three saffron steam strokes — inline SVG, 24px grid, see canvas.

## Signature
Wax-print band (concentric circles + zigzag, SVG, `aria-hidden`) only on sign-in, device approval and empty states. Never on dense working surfaces.

## Screens
- Kitchen · List: header (pot name, view tabs List/Board/Activity/+ View, filter chips, ⌘K, New task) · groups by status with row = id · title · progress bar + n/m items · label · note · holder · lease timer/epoch · right rail "From the kitchen" (events).
- Kitchen · Board: 5 columns (ready, on the fire, waiting, served, tossed), compact cards with progress; header counts and column totals are the same numbers (`bucket_sql`).
- Sign-in / Approve device: indigo left panel (wax band, logo, headline, signed-in-as + Log out) · right: code in Space Mono 46px, details card, Approve (primary) / Deny.
- Cooks: card with avatar letter, kind/runtime, live state, current task, MCP config, Rotate token.

## Lists
- Every list reads a `ListSource` (`lib/listbox.ts`); the per-procedure hooks are in `lib/sources.ts` (`useTeamsSource(q)`, `useLabelsSource(pot, {q, bucket})`, ...), thin wrappers over `useListQuery` (cursor paging, debounced server search). Never fetch "all" rows for a select.
- Pickers: `VirtualCombobox` for data-driven selects (`ui/select` only for fixed enums), `VirtualMenu` for data-driven menus, `VirtualCommand` for the palette, `TagInput` v2 for tags, `MentionPopup` for `@` mentions. All are virtualized, with listbox roles and `aria-activedescendant` keyboard navigation.
- Filters: `FilterBar` (search + facet chips) over `useUrlFilters`, so a filtered list is a link.
- Pages: `ListPage` (title, filters, states: loading, empty, error, loading more, capped, end). Pages scroll with the window; internal scroll only inside popovers and dialogs.
- One page, one scroll, so only the LAST list on a page loads more as its end scrolls into view. Every list that has more content below it (another list or section) reads its next page on an explicit "Show more" button. The shared list parts take `more: "scroll" | "button"` (default `"scroll"`): `SourceList`, `StationList`, `CloudRunList`, `ReviewsList`, `MenuCards`, `UsagePanel` (default `"button"`, it sits above other sections), and `listFooter(source, more)` / `loadMore(source, more)` in `components/list-footer.tsx` for a hand-built `VirtualList`. A page of one list (`ListPage`) is its own last list. Lists side by side count in reading order (DOM order).
- Server-backed menus and chips read only while open: pass the picker's `onOpenChange` to a `useListQuery`'s `enabled`, so a page of filter chips or a views menu does not read every list up front. A hook for a page's filters takes a flag for whether the page shows them at all.

## Mobile and tablet (mobile first)

References (Refero): Linear iOS (grouped status lists, issue detail, display-options bottom sheet), Notion iOS (board as swipeable columns), Monday/Wrike web (list with a detail side panel on wide screens).

**Tiers.** Write the phone layout first, then add `md:` and `lg:`.
| tier | width | navigation | overlays |
|---|---|---|---|
| phone | < 768 (`max-md`) | top bar (pot switcher, search, +) and bottom tabs: Board · Pass · Questions · Cooks · More | dialogs become bottom sheets below 640px; the task sheet is full screen |
| tablet | 768–1023 (`md`) | 72px icon rail: pots by key, Cooks, Fires, Teams, admin, search, account | dialogs centred, side sheets 3/4 wide (max 24rem) |
| desktop | ≥ 1024 (`lg`) | the 256px sidebar | as before |

In JS, `useIsPhone()` and `useIsDesktop()` come from `src/lib/media.ts`. Prefer CSS when it can do the job.

**Phone rules.**
- The top bar already names the pot. Pages under `/pots/:key/*` drop their big title on phones and keep one compact line of counts or actions.
- Pot tabs outside the bottom bar (Activity, Pantry, Menus, Files, About) are in More. Saved views stay reachable from the Board tab.
- Touch targets are at least 44px (`pointer-coarse:` sizes on `Button`). Rows are at least 48px. Never rely on hover alone: a hover-only action needs a visible or long-press path on touch.
- Inputs are 16px on phones, set globally so iOS doesn't zoom on focus.
- Anything fixed to the bottom clears the tab bar: use the `bottom-tabbar` and `pb-tabbar` utilities. Respect `env(safe-area-inset-*)`, since the page uses `viewport-fit=cover`.
- Lists over tables: a table on a phone becomes stacked cards or two-line rows (title on line one, mono meta on line two). Nothing scrolls sideways but code blocks and wide markdown tables, inside their own box.
- Board: from 640px up, all five columns always side by side (19rem each, sticky name and count), filling the screen below the pot header; the row scrolls sideways inside the board when they do not fit and every column scrolls up and down on its own. Phones: one column per screen in a snapping strip (swipe), with a header strip of the five names and counts above it that jumps to a column when tapped and lights the one in view (`?col=` in the URL).
- Sheets and dialogs scroll inside themselves (`min-h-0 flex-1 overflow-y-auto`). Keep primary actions in a sticky footer.

**One scroll.** Every page scrolls vertically with the window, at every screen size, and never horizontally. Virtualized lists use `scroll="window"`. The exceptions are scrollers that are the point of their screen: the chat thread (bottom-anchored) and the board. The board fills the viewport below the pot header (its top is measured into `--fill-top` by `useFillTop`, the height is CSS) so the page itself does not scroll past it; inside it a row of columns scrolls sideways (scroll-snap) and each column is its own vertical scroller (`VirtualColumn scroll="own"`) that pages as its end is reached. Also scrolling inside themselves: popovers and menus, dialogs and sheets over the page, and the app sidebar (navigation chrome). `scripts/ui/page-scroll.mjs` checks this on the Storybook stories at 320, 375, 768, 1280 and 1920 px (the page never scrolls sideways; the board's scrollers are allowed, and the board must fit the screen).

**Tablet.** Use the space for list and detail side by side where it helps (task list plus detail, pass plus thread). Keep desktop density, but with touch-size targets.

**Native app.** The Tauri v2 shell (`apps/mobile`) loads the console from the server. `html[data-native]` marks the app for app-only tweaks, such as no overscroll bounce.

**Checks.** `scripts/ui/devstack.sh` runs the stack and `scripts/ui/shots.mjs` screenshots pages at phone, tablet and desktop sizes. `scripts/ui/interact.mjs` clicks through a flow.
