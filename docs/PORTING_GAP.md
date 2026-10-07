# Breakscale Desktop — Porting Gap

Status of the desktop port against the web app at **upstream main `d2c876e0`** (2026-09-29,
last push 2026-10-05): https://github.com/xevrion/breakscale

**Local web reference** (`../src`, this repo's sibling) is pinned at upstream
**`a3254c0e`** (2026-08-29) — verified by git blob-hash match on `src/App.tsx` and
`src/sim/engine.ts`. Anything marked *(delta)* landed upstream after that commit and must be
read from **upstream main**, not from the local copy.

Upstream is 80 commits ahead of the base. This file is the porting backlog; the phase plan
lives in the release checklist.

---

## P0 — Engine / sim parity (the Rust engine is missing these) *(delta)*

| Upstream commit | PR | What | Why it matters |
|---|---|---|---|
| `79254ce0` | #56 | **Load balancer admits through its pool** — `onAdmit` stops returning `passthru`, `capacity`/`instances`/`queueLimit` go through ordinary slot and queue discipline | Today an lb sized for 2 concurrent calls carries ~3000 rps, `waiting` stays 0, autoscaler `instances` binds nothing. Biggest correctness hole. |
| `d1c9fee2` | #57 | **Count a timeout once, and let the failure rate see it** — `onTimeout` solely owns the `timeouts` counter; `resolve` owns root `totalFailed`; `errorRate` includes timeouts | Double-counted failures (up to 26% phantom loss); the canvas "failing" cell read 0% while traffic died. |
| `c24134ad` | #33 | **Client arrival streams stable across topology edits** — preserve each retained client's scheduled arrival event; generation tokens kill stale events on remove/retype; reset restores generation state | 10 structural edits moved a 51.1 rps client to 488.9 measured rps. Breaks the determinism contract. |
| `6df7fb9f` | #48 | **Preserve autoscaled `instances` across hot swaps** — `authoredInstances` map, recorded like `capacity`/`shardCapacity`; `reset()` rewinds it; `buildNodes` keeps a controller-written value that diverges from authored | Structural edits revert live fleets; reset replays a scaled run from the wrong count. |
| `351327c4` | #77 | **Bulkhead acquire-queue mode** *(feature)* — bounded acquire queue, acquire timeout, pool metrics, inspector controls, glossary entry, wire-format bit; default stays immediate rejection | Models connection-pool exhaustion (wait → acquire-timeout → retry amplification). Biggest new sim feature. |
| `870b70d0` | #58 | Bound the instance count the engine acts on (`effectiveInstances` clamp) | Huge / non-numeric `instances` takes the app down on first snapshot. |
| `0820b8c0` | #60 | Bound data-tier fleet counts (`shardCount`, `replicaCount`, `shardCapacity`) — `clampInt` | Same shape, one layer down. |
| `dcb6980c` | #62 | Bound the rendition ladder the transcoder acts on | Malformed designs crash the sim. |
| `91e06bf8` | #61 | Bound the partition count a broker will act on | Same. |
| `1f4c63b5` | #63 | Region switch keeps routing when its numbers are NaN | NaN config kills routing. |
| `ea11c5b3` | #64 | Autoscaler keeps controlling when a knob is NaN | NaN config kills scaling. |
| `7cc1fea9` | — | Count an arrival the engine had no room for | Admission accounting. |
| `7e285598` | #26 | `watchedUnscalable`: `decorateStats` reports an autoscaler watching a fleetless kind (blob store) distinctly; Inspector says so in words | Sim + UI copy: "adding servers is not the fix" is the lesson. |

## P1 — App features *(delta)*

| Upstream commit | PR | What | Desktop status |
|---|---|---|---|
| `3ce685bd` | #76 | **WYSIWYG note editing** — `textWysiwyg.ts` (Excalidraw-style imperative textarea), `autoResize` notes (wire bit 12), `annotationLayout` extraction, note frame chrome, webfont metrics epoch | **Missing.** Replaces the base note editor — the annotation format toolbar (S/M/L, B/I/U, fonts, tones) must be built on top of this, not on the old editor. |
| `eb163673` | #71 | **Inspector "suggest one fix"** — one grounded suggestion per kind when headroom < 1.0x, gated on real panel state (`src/content/suggestions.ts`) | **Missing.** |
| `1db4ac61` | #37 | **G toggles snap-to-grid** — preference actually wired into all four drag paths via `snapsToGrid()`, Ctrl = momentary loosen-only override, Shortcuts entry | Desktop reads `settingsStore.snapToGrid` (`Canvas.svelte:448`); **verify** Ctrl override, G key, Shortcuts row, palette-drop honours it. |
| `4fd46c47` | #65 | Never mint a node id that already exists; read the live topology mirror on add | **Verify** desktop add path. |
| `254a1b51` | — | Hold the camera still when a component lands | **Missing.** |
| `dc9c1a07` | — | Palette row as a card, not a screenshot of the row | **Missing.** |
| `e0241b03` | #54 | Keep a design this build cannot open instead of erasing it (`savedDesigns` rejected-design guard) | Port to the desktop Designs/backup storage path. |
| `6dd8c0e9` | #43 | Share `d2.`: strip default configs, round coordinates, decode `d1.`/`d2.` | Port decoder parity if the desktop should open web share links. |
| `b9590433` | #75 | Compact wire format (`src/share/wire.ts`, 894 lines) | Same as above. Watch open PR **#88** (append-only wire tables + bulkhead). |
| `84be515a` | — | Link store (server-side short links) | Web infra — skip. |
| `80dc6020` | #34 | Offered-load slider stays still while saving | Cosmetic — port if the layout matches. |
| `31101758` | #42 | Traffic-load and spacer style fixes | Cosmetic. |
| `1b804f42` | — | Canvas controls stay on the charts strip in short windows | Cosmetic. |
| `294fc9e3` | #49 | Theme toggle visibility in Settings | Cosmetic. |

## Base-era gaps (already covered by the phase plan)

These exist in the local web reference (pre-`a3254c0e`) but were declared out of scope in the
original desktop port (`Canvas.svelte:57`, `:2036`):

- **TrafficControl live-readout island** in the header (offered load, hero p99, goodput,
  errors, dropped) — desktop's `.app-island-load` CSS is dead (`shell.css:100`).
- **Inspector close hardening** — `--bar-clear` is hardcoded 80px; web measures it in JS.
- **Right-click select** — web is inert; a desktop requirement (explicit user request).
- **Edit-while-running feedback + tests.**
- **Reset = `buildNodes(null)` parity** — desktop `reset()` carries old node state forward
  (`engine.rs:2818` → `build_nodes` reuses `previous`), so Reset looks like a no-op; web
  `engine.ts:1054` passes `null`.
- **Group-drag multi-selection** — web `promote()` shared-delta logic (`Canvas.tsx:3902-3973`,
  `:4220-4243`).
- **Undo / redo** — web `src/history.ts` SessionHistory; desktop already ships dead CSS for the
  cluster and receipt (`shell.css:918`, `:931`).
- **Zoom-to-fit** — web `fitTo` + `FIT_*` constants, Shift+1 / Shift+2, readout-as-fit-button.
- **Edge-label collision avoidance** — web `labelDyById`: 16px buckets, 10px downward stagger,
  participants = edges that render labels, `showEdgeLabels` gated at zoom >= 1; desktop always
  draws `labelDy = 0` (`Canvas.svelte:1606`, `:2051`).

## Out of scope (note only)

- **MCP server** (~15 commits: hosted Vercel function, tool directory, canvas in chat) and the
  **VS Code extension** (`bcb5fb39`) — separate products.
- Docs site per example (`e4af2459`), SEO/README/privacy/sponsor work, dependency and CI bumps,
  star-count badge (`2bb00fae`, optional).
- Open upstream PRs to watch: **#90** (queue-panel hint stability), **#88** (wire tables
  append-only + bulkhead), **#73** (visual regression tests).

## Desktop-only extras (keep — the desktop is ahead here)

- System-design editors (Architecture / Low-Level) and the export panel.
- Glossary overlay panel, backup/restore in Settings.
- (Everything else on the desktop matches or extends the web base.)

---

*Research method: `gh api` compare `a3254c0e...main` (80 commits), PR bodies for #33–#77,
blob-hash pinning of the local reference copy, and static tracing of each reported bug to its
root cause in the desktop sources.*
