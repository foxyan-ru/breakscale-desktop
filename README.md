# Breakscale desktop

A discrete-event system-design simulator, packaged as a Tauri 2 desktop
application with a SvelteKit frontend and a Rust backend. This is a
migration of the Breakscale web app; the full architecture rationale,
module-by-module mapping and dependency justification live in the parent
project's `MIGRATION_PLAN.md` (not part of this repository). This
file covers what a maintainer actually needs day to day: setup, build,
packaging, signing, and — since no build or test was run while producing
this migration (see "Known limitations" below) — exactly what to verify
before trusting it.

## Architecture overview

```
├── src/                      SvelteKit frontend, adapter-static, SSR off
│   ├── routes/                 +layout.svelte, +page.svelte — the whole app is one page
│   └── lib/
│       ├── domain/              TS types mirroring the Rust wire format (camelCase JSON both sides)
│       ├── api/                  typed invoke() wrappers — the ONLY files that call @tauri-apps/api directly
│       ├── state/                 Svelte 5 rune stores (simulation, topology, ui, settings, theme, sysdesign, vendor-sizes)
│       ├── theme/                 design tokens / contrast / applyTheme, ported from src/theme
│       └── components/            one directory per panel: canvas, palette, inspector, metrics, cost,
│                                  trace, vendor, glossary, challenges, sysdesign, shell (menus/dialogs)
└── src-tauri/                 Rust backend
    ├── src/
    │   ├── sim/                  the ported simulation engine: types, random, heap, engine, behaviour/*,
    │   │                         challenge(s), presets, glossary — pure domain, no file I/O (see AGENTS.md's
    │   │                         "no React, no DOM, no I/O" rule, which this module inherits)
    │   ├── vendors/                cloud vendor specs + cost/capacity-derivation arithmetic
    │   ├── persistence/             design_file (.breakscale format), saved_designs (the named shelf), backup
    │   ├── sysdesign/                NEW: high/low-level architecture model, derivation from a topology,
    │   │                             validation, export (JSON/YAML/Markdown/Terraform)
    │   ├── commands/                 thin #[tauri::command] wrappers, grouped by domain — no logic lives here
    │   ├── state.rs                  the one live simulation Engine + its background tick thread
    │   └── error.rs                  AppError → the JSON shape every command rejection carries
    └── data/                      presets, glossary and vendor specs as JSON, embedded via include_str!
```

**Why the simulation runs in Rust, not the WebView.** The engine ticks on a
fixed interval and polls at 10Hz regardless of what the UI is doing; a
background OS thread owns the one live `Engine` (behind
`Arc<Mutex<Option<Engine>>>`) and pushes `sim://snapshot` events to the
frontend, so the simulation keeps advancing even while the webview is
mid-reflow, and the UI thread is never blocked waiting on it. See
`src-tauri/src/state.rs`.

**Why the design-file format is unchanged.** Every Rust struct that
crosses the Tauri IPC boundary or gets written to a `.breakscale` file is
`#[serde(rename_all = "camelCase")]`, matching the web app's JSON
field-for-field. A `.breakscale` file saved by either app opens in the
other — see the parent project's MIGRATION_PLAN §6.

## Prerequisites

- [Bun](https://bun.sh) — the whole project standardises on it.
- [Rust](https://rustup.rs) (stable channel, 1.77+).
- The [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for
  your OS:
  - **Windows**: Microsoft C++ Build Tools (Visual Studio Build Tools,
    "Desktop development with C++" workload) and the WebView2 runtime
    (preinstalled on current Windows 10/11).
  - **macOS**: Xcode Command Line Tools (`xcode-select --install`).
  - **Linux**: `webkit2gtk-4.1`, `libappindicator3`, `librsvg2`, `patchelf`,
    and a GTK 3 dev package — exact package names in
    `.github/workflows/desktop-release.yml`'s Linux dependency step.
- `@tauri-apps/cli`, installed via `bun install` below (no separate global
  install needed).

## Setup

```bash
git clone https://github.com/rainman456/breakscale-desktop.git
cd breakscale-desktop
bun install
```

**App icons** are checked in, generated once from
[`static/favicon.svg`](static/favicon.svg) (see
[`src-tauri/icons/README.md`](src-tauri/icons/README.md)). To re-generate
after changing the source art:

```bash
bun x @tauri-apps/cli icon static/favicon.svg
```

## Development

```bash
bun run tauri dev
```

Starts the Vite dev server (port 1420) and opens it in a native window
with hot reload. `bun run dev` alone starts only the Vite server in a
regular browser tab, useful for quick UI iteration but without any
`@tauri-apps/api` calls working (there is no Tauri runtime backing a plain
browser tab).

## Building a desktop executable

```bash
bun run tauri build
```

Produces a native installer/bundle under `src-tauri/target/release/bundle/`:

| OS | Output |
|---|---|
| Windows | `bundle/msi/*.msi` and `bundle/nsis/*.exe` |
| macOS | `bundle/macos/*.app` and `bundle/dmg/*.dmg` |
| Linux | `bundle/deb/*.deb`, `bundle/appimage/*.AppImage`, `bundle/rpm/*.rpm` (target availability depends on the build host's distro) |

The release workflow builds Windows with `--bundles nsis` only, so the
`.exe` (not the MSI) is the Windows artifact we ship. Cross-compiling a
Windows/Linux bundle from macOS (or vice versa) is not generally supported
by Tauri's bundler — build each target on that OS, or use the CI matrix in
`.github/workflows/desktop-release.yml`, which builds all three on their
native runners.

## Signing, packaging and distribution

Unsigned builds work for local testing; every OS below will warn or block
an end user opening an unsigned app from outside a package manager.

### Windows

Code-sign the `.exe`/`.msi` with `signtool` and an Authenticode certificate
(an EV cert avoids SmartScreen's "unknown publisher" prompt; a standard OV
cert still triggers it until the binary has enough install reputation).
Tauri's bundler will sign automatically if you set, before `tauri build`:

```bash
TAURI_SIGNING_PRIVATE_KEY=...        # PFX-derived signing key, see Tauri's Windows code-signing guide
TAURI_SIGNING_PRIVATE_KEY_PASSWORD=...
```

or sign the produced `.exe`/`.msi` manually afterward with `signtool sign`.

### macOS

Notarization is required for Gatekeeper to allow the app on another
machine without a right-click override. Set, before `tauri build`:

```bash
APPLE_CERTIFICATE=...                # base64-encoded .p12 Developer ID Application certificate
APPLE_CERTIFICATE_PASSWORD=...
APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
APPLE_ID=you@example.com
APPLE_PASSWORD=...                   # an app-specific password, not your Apple ID password
APPLE_TEAM_ID=...
```

Tauri codesigns and submits for notarization as part of `tauri build` when
these are set — locally, or in CI's "Build Tauri app" step, which passes
the same names as Actions secrets (see the
[Tauri macOS signing guide](https://v2.tauri.app/distribute/sign/macos/)).

### Linux

No OS-level signing step; a `.deb`/`.rpm` can optionally be signed with a
GPG key for a package repository, and an `.AppImage` can be signed with
its own embedded-signature mechanism if you distribute it standalone. Not
configured in this migration — add if/when a distribution channel needs it.

## GitHub Actions release workflow

[`.github/workflows/desktop-release.yml`](.github/workflows/desktop-release.yml)
builds all three platforms with `bunx tauri build` on a `desktop-v*` tag
push, uploads each platform's installers as artifacts, and — for tag
builds only — attaches them to a DRAFT release on
`rainman456/breakscale-desktop`. Every action in this repository's
workflows is SHA-pinned (both files carry the tag each pin corresponds
to; refresh with `git ls-remote` when bumping).

The workflow is expected to run in the fork
(`foxyan-ru/breakscale-desktop`): push the tag there, and push the same
tag to this repository so the draft release lands on the right commit.
The release step authenticates with the `RAINMAN_TOKEN` secret — a
`rainman456` PAT with `repo` scope, pasted into the fork's Actions
secrets — because a fork's own `GITHUB_TOKEN` cannot publish to the
parent repository. `workflow_dispatch` runs stop after the artifacts, so
a manual run never touches a release. Signing secrets (above) are
optional: unsigned artifacts still upload, they just are not fit for
public distribution.

## Known limitations & manual verification steps

**No build, install, or execution command was run while producing this
migration** (an explicit constraint on the work, not an oversight) — every
file was written and statically reviewed, never compiled. Treat this as a
complete first draft awaiting its first `cargo check` / `bun run tauri
dev`, not as a verified build. In the order you should check them:

1. **`cargo check` inside `src-tauri/`, then `bun run tauri dev`.** Fix
   whatever the compiler and `svelte-check` find first — many independent
   agents wrote against a shared written contract without being able to
   see each other's final code, so naming or minor signature drift between
   a Rust command and its frontend caller is the most likely class of
   first error. Search the frontend for `NOTE(integration)`/
   `TODO(integration)` comments first; several were left deliberately
   where an agent invented a command contract for the integration pass to
   implement, and most already have a matching Rust command in
   `src-tauri/src/commands/` — cross-check spelling/argument names between
   the two sides if a call fails.
2. **Determinism of the simulation engine.** `src-tauri/src/sim/random.rs`
   (the seeded RNG) and `src-tauri/src/sim/engine.rs` (the event loop) were
   ported operation-for-operation from the TypeScript originals, but
   nothing in this session could confirm they produce byte-identical
   output for the same seed and topology — the property the web app's own
   `AGENTS.md` treats as a hard contract. Add a cross-language fixture
   (fixed seed, N ticks, compare `snapshot().system`) before trusting
    replay parity. See the parent project's MIGRATION_PLAN §9.
3. **Canvas interaction fidelity.** `src/lib/components/canvas/Canvas.svelte`
   reimplements the web app's hand-rolled SVG hit-testing/gesture system
   using Svelte's own event model rather than transliterating the original
   React-specific pointer-capture workarounds. Functional but at reduced
   fidelity in several places the porting agent documented inline and in
   its own report — worth a manual pass with a mouse, a trackpad, and (if
   available) a touchscreen: node placement/drag, edge wiring, pan/zoom,
   multi-select/marquee-select/clipboard/rename (implemented in a later
   pass — see "What is NOT ported" below for what of this area is still
   actually missing), annotation resize (corner/font-scale handles not
   implemented, side handles are), and the minimap's click-to-jump.
4. **Data fidelity of the 23 presets and 100 glossary entries.** Converted
   mechanically from the TS literals to JSON and spot-checked
   programmatically (node/edge referential integrity, required-field
   presence, field-count diffs against the source's own field list) rather
   than diffed value-by-value against `src/sim/presets.ts`. Low risk, not
   zero — worth a value-level diff before shipping if a preset's exact
   tuned numbers matter to a downstream test or challenge.
5. **App icons are placeholder art.** The checked-in set was generated from
   `static/favicon.svg` and is valid for bundling, but it is the favicon
   scaled up rather than a dedicated 1024×1024 app icon — re-run
   `bun x @tauri-apps/cli icon <source.png>` with proper art before a
   public release (see "Setup" above).
6. **Vendor size → engine-capacity derivation UI.** `VendorPanel.svelte`
   inlines the `deriveFromSize`/`isSizedKind` arithmetic directly (a port
   of `src/content/vendors/derive.ts`) rather than calling into the Rust
   `vendors::derive` module, which exists but has no command wrapping it
   yet — both copies of this logic must be kept in sync if either changes;
   consider collapsing to one by adding a `vendors_derive_capacity`
   command and switching the frontend to call it.
7. **Settings/layout persistence is wired but not yet consumed.**
   `settings_load`/`settings_save`/`layout_load`/`layout_save` commands
   exist (`src-tauri/src/commands/settings.rs`), but
   `src/lib/state/settings.svelte.ts` still has its original
   `TODO(integration)` comment for actually calling them on startup/change
   — the preference store works in-memory for the lifetime of one session
   but does not yet survive a restart. A small follow-up: call
   `settings_load` in the root layout's `onMount` and `settings_save`
   whenever `settingsStore` changes (e.g. via a `$effect`).
8. **Design compatibility edge case.** `NodeConfig`'s six fleet-sizing
   fields (`replicaCount`, `replicationLagMs`, `readFraction`,
   `shardCount`, `shardCapacity`, `hotKeyFraction`) are non-optional in the
   Rust struct, matching the TS interface's own declared shape — but the
   web app's runtime `isTopology()` guard never actually checks those six
   fields are present (it only validates 9 others). A `.breakscale` file
   saved before those fields existed, or hand-edited to omit them, will
   open in the web app but be rejected by this app's stricter
   `serde`-derived parser. Low practical risk (every preset/saved-design
   path in this codebase always includes them), flagged for completeness
   — see `persistence::design_file`'s module doc for the full note.

## What is NOT ported (out of scope for this migration)

- Alt-drag-duplicate and Ctrl+D duplicate on the canvas (multi-select,
  marquee-select, node rename and system clipboard copy/cut/paste ARE
  implemented — see `Canvas.svelte`'s header and closing comments).
- Group-drag-move of a multi-selection: dragging one member of a
  multi-selection selects and moves only that one node.
- Undo/redo and zoom-to-fit on the canvas.
- The annotation text editor's bold/italic/underline/font/tone toolbar.
- Pixel-identical edge-label collision avoidance at a symmetric fan-out.

Each is a self-contained follow-up; none blocks the app from running.
