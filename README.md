# Breakscale desktop

A discrete-event system-design simulator, packaged as a Tauri 2 desktop
application with a SvelteKit 2 + Svelte 5 frontend. This is a port of the
Breakscale web app — the web app's source is the spec every feature is
checked against.

**The simulation engine runs in the webview, not in Rust.** Earlier
releases (`desktop-v0.1.0` through `v0.2.2`) ran the engine in a Rust
background thread and pushed snapshots to the frontend over Tauri
IPC/events at ~10Hz; that turned out to be the actual source of reported UI
lag and high CPU usage, since every tick paid a JSON-serialization round
trip across the IPC boundary. `desktop-v0.3.0` moved the engine into the
frontend as TypeScript — the same engine the web app runs, not a
reimplementation of it — so there's no IPC in the simulation's hot path at
all anymore. Rust/Tauri is now a thin native shell: window chrome, file
dialogs, persistence, and a desktop-only system-design export feature (see
below).

## Architecture overview

```
├── src/                      SvelteKit frontend, adapter-static, SSR off
│   ├── routes/                 +layout.svelte, +page.svelte — the whole app is one page;
│   │                           +page.svelte's onMount also starts the simulation loop
│   └── lib/
│       ├── sim/                  THE SIMULATION ENGINE, in TypeScript — a direct, byte-for-byte
│       │                         copy of the web app's own src/sim/*.ts (engine, behaviour/*,
│       │                         types, presets, challenges, heap, random, annotations). Zero
│       │                         React/DOM/Tauri dependency, so it ported with no translation.
│       ├── state/                 Svelte 5 rune stores. simulation.svelte.ts owns the one live
│       │                         Engine instance and a requestAnimationFrame loop, publishing
│       │                         snapshots via $state.raw at 10Hz (see its own header comment —
│       │                         there are real reference-identity gotchas, since the engine
│       │                         reuses some internal containers across snapshots for
│       │                         performance). topology.svelte.ts is the diagram's source of
│       │                         truth and calls straight into the engine synchronously.
│       ├── domain/              TS types mirroring the Rust wire format for what's STILL on IPC
│       │                         (persistence, sysdesign, vendors) — camelCase JSON both sides.
│       │                         sim-types.ts/annotations.ts are now thin re-exports of lib/sim's
│       │                         own types, not a separate copy.
│       ├── api/                  typed invoke() wrappers — the ONLY files that call
│       │                         @tauri-apps/api directly. No simulation calls here anymore,
│       │                         and no events at all (no more listen()) — only persistence,
│       │                         dialogs, system-design export, and one-time static data
│       │                         (presets/challenges/glossary/vendors).
│       ├── theme/                 design tokens / contrast / applyTheme, ported from src/theme
│       └── components/            one directory per panel: canvas, palette, inspector, metrics, cost,
│                                  trace, vendor, glossary, challenges, sysdesign, shell (menus/dialogs)
└── src-tauri/                 Rust backend — a thin native shell, not where simulation runs
    ├── src/
    │   ├── sim/                  DATA ONLY: types.rs (wire shapes), presets.rs, challenge(s).rs,
    │   │                         glossary.rs. No engine, no behaviour/, no heap/random — those
    │   │                         were removed in desktop-v0.3.0 along with the Rust tick thread.
    │   ├── vendors/                cloud vendor specs + cost/capacity-derivation arithmetic
    │   │                         (cost.rs/derive.rs are currently dead code — the frontend has
    │   │                         its own copy; a future cleanup, not done yet)
    │   ├── persistence/             design_file (.breakscale format), saved_designs (the named shelf), backup
    │   ├── sysdesign/                high/low-level architecture model, derivation from a topology,
    │   │                             validation, export (JSON/YAML/Markdown/Terraform) — genuinely
    │   │                             desktop-only (no upstream equivalent); stays in Rust because
    │   │                             YAML export needs serde_yaml and Terraform export needs real
    │   │                             filesystem writes, neither available to a browser tab
    │   ├── commands/                 thin #[tauri::command] wrappers, grouped by domain — no logic lives here.
    │   │                             26 commands total; there is no commands/sim.rs anymore.
    │   └── error.rs                  AppError → the JSON shape every command rejection carries
    └── data/                      presets, glossary and vendor specs as JSON, embedded via include_str!
```

**Why the design-file format is unchanged.** Every Rust struct that
crosses the Tauri IPC boundary or gets written to a `.breakscale` file is
`#[serde(rename_all = "camelCase")]`, matching the web app's JSON
field-for-field. A `.breakscale` file saved by either app opens in the
other.

**Known persistence gaps** (real bugs, not new in this architecture
change): settings and window layout don't survive a relaunch (the
`settings_load`/`settings_save`/`layout_load`/`layout_save` commands exist
but nothing calls them yet); saved system-designs can't be reopened from
the UI and every save overwrites the same file; backups aren't compatible
with the web app's `localStorage`-keyed format.

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

The app builds, runs, and is CI-verified (`bun run check`/`test` on Ubuntu,
`cargo test` on Windows) on every push — this section is real, current
limitations, not a first-draft disclaimer.

1. **Settings/layout persistence is wired but not yet consumed.**
   `settings_load`/`settings_save`/`layout_load`/`layout_save` commands
   exist (`src-tauri/src/commands/settings.rs`), but nothing on the
   frontend calls them on startup/change yet — the preference store works
   in-memory for the lifetime of one session but does not survive a
   restart.
2. **Saved system-designs can't be reopened**, and every save overwrites
   the same file (`sysdesignStore.load()` is never called from the UI;
   every document is created with a hardcoded `id: "unsaved"`).
3. **Backups aren't compatible with the web app.** Desktop keys backup
   data by file name (`saved_designs.json`, …); the web app keys the same
   data by `localStorage` key (`breakscale.designs.v1`, …) — a backup made
   by one won't restore on the other, and neither backs up a "session" the
   way the web app does (so desktop always boots the first preset, never
   "where you left off").
4. **`vendors/cost.rs` and `vendors/derive.rs` (Rust) are dead code** — the
   frontend (`VendorPanel.svelte`, `components/cost/cost.ts`) has its own
   copy of the same vendor-size → capacity arithmetic. Harmless (both
   copies are kept in sync by virtue of neither changing independently
   yet) but worth collapsing to one, ideally by deleting the Rust side in
   favor of upstream's own `src/content/vendors/*.ts`.
5. **App icons are placeholder art.** The checked-in set was generated from
   `static/favicon.svg` — valid for bundling, but it's the favicon scaled
   up rather than a dedicated 1024×1024 app icon. Re-run
   `bun x @tauri-apps/cli icon <source.png>` with proper art before a
   public release (see "Setup" above).
6. **Installer version strings read `0.1.0`.** `package.json` and
   `src-tauri/tauri.conf.json`'s `version` field were never bumped to
   track the `desktop-v*` release tags — cosmetic only (the installers and
   their embedded binaries are correct for the release they ship with),
   but worth fixing before this matters to anyone checking "Help → About".
7. **Design compatibility edge case.** `NodeConfig`'s six fleet-sizing
   fields (`replicaCount`, `replicationLagMs`, `readFraction`,
   `shardCount`, `shardCapacity`, `hotKeyFraction`) are non-optional in the
   Rust struct, matching the TS interface's own declared shape — but the
   web app's runtime `isTopology()` guard never actually checks those six
   fields are present (it only validates 9 others). A `.breakscale` file
   saved before those fields existed, or hand-edited to omit them, will
   open in the web app but be rejected by this app's stricter
   `serde`-derived parser. Low practical risk (every preset/saved-design
   path in this codebase always includes them) — see
   `persistence::design_file`'s module doc for the full note.

## What is NOT ported

Confirmed, as of `desktop-v0.3.0`, by diffing this repo's components against
upstream's `src/components/*.tsx` — these are real gaps, not desktop-only
extras, and none of them block the app from running:

- **`PanelResizer`** — upstream lets you drag-resize the side panels; fixed
  widths here.
- **Share links** (`Share.tsx`/`share.ts`/`share/wire.ts`) — generating a
  shareable URL encoding a topology. Not applicable 1:1 to a desktop app
  (no URL to share), but the underlying encode/decode isn't ported either.
- **`imageExport.ts`** — exporting the canvas as a PNG/SVG image.
- **`useCoarsePointer`/`presence.ts`** — upstream's touch/coarse-pointer
  affordances and multi-cursor presence indicators.
- **`useGithubStars`** — a GitHub star-count badge; cosmetic, web-only.
- **Session restore** (`breakscale.session.v1`) — the web app restores
  "where you left off" across a browser reload; desktop always boots the
  first preset (see "Known limitations" above — also entangled with the
  backup-format gap).

Everything else in upstream's component tree (undo/redo, zoom-to-fit,
multi-select/marquee/clipboard, the annotation format toolbar, right-click
node select, group-drag, the bulkhead/autoscaler/region control-plane
nodes, etc.) is ported and in use.
