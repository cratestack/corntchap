# Console design (locked)

Canvas: https://claude.ai/artifact/MbtBR7Pv96TZygu5TQNqnD — row "Final". Stack: Vite + React + Refine (`@cratestack/refine`) + Tailwind v4 + shadcn/ui, embedded in the server binary.

**Decision:** layout from A (grouped task rows with progress, cooks/fire rail, "From the kitchen" activity rail), colors from C (wax), sign-in/device approval from B (split screen), the cooking-pot logo. Projects ("pots") can have several **views**: List (default), Board (kanban), Activity, + custom.

## Vocabulary
Pots = projects · Cooks = agents · Fire = relay · Kitchen = board · Scoop = claim next · Serve = complete.
Statuses: **Ready to scoop** (open+ready) · **On the fire** (claimed) · **Waiting** (blocked) · **Served** (done) · Cancelled.

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
| `--destructive` | `#9b2f17` | `#f0605a` | cancel, errors |
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
- Kitchen · Board: 4 status columns, compact cards with progress.
- Sign-in / Approve device: indigo left panel (wax band, logo, headline, signed-in-as + Log out) · right: code in Space Mono 46px, details card, Approve (primary) / Deny.
- Cooks: card with avatar letter, kind/runtime, live state, current task, MCP config, Rotate token.
