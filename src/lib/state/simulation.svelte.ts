/**
 * Svelte 5 rune store for live simulation state, and the ONLY place that
 * owns the `Engine` instance.
 *
 * WHY THIS REPLACED THE Rust-backed VERSION: the engine used to run in a
 * Rust background thread, ticking at a fixed rate and pushing a full
 * `SimSnapshot` to the frontend over a Tauri event (`sim://snapshot`) at
 * ~10Hz -- every tick serialised the whole state across the IPC/JSON
 * boundary. Tauri's own docs say plainly that its event system "is not
 * designed for low latency or high throughput situations" and that
 * "event payloads are always JSON strings making them not suitable for
 * bigger messages" (https://v2.tauri.app/develop/calling-frontend/). That
 * was the real source of the UI lag/high-CPU complaints, not anything
 * fixable by throttling harder on one side of the bridge. The web app
 * this is ported from (`src/App.tsx`) never pays that cost: `Engine` runs
 * directly in the same process as the UI. This file is the Svelte
 * equivalent of that `App.tsx` loop (constants, shape, and all), with
 * Tauri kept ONLY for native file I/O (dialogs, saved designs, backups,
 * system-design export) -- none of which run every frame.
 *
 * ENGINE OWNERSHIP: `engine` below is a plain module-level variable, never
 * wrapped in `$state`. The engine mutates its own internal containers in
 * place for performance (object pools, reused snapshot buffers -- see
 * `$lib/sim/engine.ts`'s own header), and Svelte 5's `$state` proxy would
 * both fight that (deep-proxying a 200k-capacity request pool it was never
 * meant to observe) and buy nothing, since nothing reads the engine
 * directly -- everything goes through `simulationStore.snapshot` below.
 *
 * SNAPSHOT IS `$state.raw`, NOT `$state`: Svelte's own docs recommend
 * `$state.raw` for "large arrays and objects that you weren't planning to
 * mutate anyway, since it avoids the cost of making them reactive"
 * (https://svelte.dev/docs/svelte/$state). `engine.snapshot()` returns a
 * FRESH outer object every call (by design, so a reassignment is always a
 * real change) but reuses the same inner containers -- `nodes`, `edgeFlow`,
 * `edgeState`, `history`, `failuresByReason`, `activeFailures` -- across
 * calls (`$lib/sim/engine.ts`'s own `snapshot()` comment explains why).
 * Deep-proxying all of that ten times a second would be pure waste with no
 * payoff: consumers must read through the reassigned `snapshot` reference
 * itself (or a specific leaf like `snapshot.nodes[id]`, which IS fresh per
 * call), never key a `$derived` on `snapshot.history`/`snapshot.nodes`
 * directly -- those keep the same identity across snapshots and a derived
 * keyed on them will not see the "change" by `Object.is`.
 */

import { Engine } from '$lib/sim/engine';
import type {
  FailureKind,
  FailureOpts,
  NodeConfig,
  SimSnapshot,
  Topology,
} from '$lib/sim/types';

/** Mirrors the web app's `App.tsx` constants exactly -- this is the parity spec. */
const MAX_FRAME_MS = 100;
const SNAPSHOT_INTERVAL_MS = 100; // 10Hz, matching `SNAPSHOT_HZ = 10` upstream.

/**
 * The one `Engine` instance for the app's lifetime. Created eagerly with an
 * empty topology so every mutation function below has something to call
 * into immediately -- `+page.svelte`'s bootstrap then calls `simSetTopology`
 * with the fetched/default preset, exactly like `src-tauri/src/lib.rs`'s
 * `setup()` used to install a default engine before the webview could reach
 * it, except now there is no "before the webview can reach it": there is no
 * IPC gap to race against.
 */
let engine = new Engine({ nodes: [], edges: [] });

interface SimulationState {
  running: boolean;
  tickError: string | null;
}

function createSimulationStore() {
  let snapshot: SimSnapshot | null = $state.raw(null);
  const state = $state<SimulationState>({ running: false, tickError: null });

  return {
    get snapshot() {
      return snapshot;
    },
    set snapshot(value: SimSnapshot | null) {
      snapshot = value;
    },
    get running() {
      return state.running;
    },
    set running(value: boolean) {
      state.running = value;
    },
    get tickError() {
      return state.tickError;
    },
    set tickError(value: string | null) {
      state.tickError = value;
    },
  };
}

export const simulationStore = createSimulationStore();

/**
 * Start the rAF simulation loop. WHY A SEPARATE FUNCTION CALLED FROM
 * `+page.svelte`'s onMount, NOT auto-started here at module scope: this
 * module is imported during SvelteKit's prerender/SSR pass too (where
 * `requestAnimationFrame` does not exist), and starting a frame loop that
 * outlives whichever component asked for it would leak across navigations
 * in a way nothing here could clean up. Call the returned stop function
 * from the same lifecycle's cleanup.
 *
 * Field-for-field port of `src/App.tsx`'s loop (`MAX_FRAME_MS` clamp so a
 * backgrounded/minimized window never replays minutes of simulated time in
 * one jump, `SNAPSHOT_INTERVAL_MS` throttling the Svelte-visible publish
 * rate independently of the engine's own tick resolution).
 */
export function startSimulationLoop(): () => void {
  let cancelled = false;
  let raf = 0;
  let last = 0;
  let sinceSnapshot = 0;

  simulationStore.snapshot = engine.snapshot();

  const frame = (now: number) => {
    if (cancelled) return;
    raf = requestAnimationFrame(frame);
    const dt = last === 0 ? 0 : now - last;
    last = now;
    if (!simulationStore.running) return;
    try {
      engine.advance(Math.min(dt, MAX_FRAME_MS));
    } catch (e) {
      simulationStore.tickError = e instanceof Error ? e.message : String(e);
      simulationStore.running = false;
      return;
    }
    sinceSnapshot += dt;
    if (sinceSnapshot >= SNAPSHOT_INTERVAL_MS) {
      sinceSnapshot = 0;
      simulationStore.snapshot = engine.snapshot();
    }
  };
  raf = requestAnimationFrame(frame);

  return () => {
    cancelled = true;
    cancelAnimationFrame(raf);
  };
}

/** Publish a fresh snapshot immediately, bypassing the 10Hz throttle. Every
 * imperative mutation below calls this so a PAUSED UI still reflects the
 * edit right away, same as upstream's `applyTopology`/`handleReset`/
 * `handleStep` all calling `setSnapshot(engine.snapshot())` directly rather
 * than waiting for the next scheduled publish. */
function publishNow(): void {
  simulationStore.snapshot = engine.snapshot();
}

/** Pause or resume the loop. Purely local now -- no IPC, so no failure mode
 * to roll back on (compare the old Rust-backed version, which optimistically
 * flipped this and reverted it if `sim_set_running` rejected). */
export function setRunning(running: boolean): void {
  simulationStore.running = running;
}

/** Create a fresh engine for `topology`, optionally seeded for determinism.
 * Replaces the whole engine instance (matching `sim_new`'s old contract),
 * rather than `setTopology`'s in-place hot-swap. */
export function simNew(topology: Topology, seed?: number): void {
  engine = new Engine(topology, seed);
  publishNow();
}

/** Hot-swap the live engine's topology in place (same engine, new wiring). */
export function simSetTopology(topology: Topology): void {
  engine.setTopology(topology);
  publishNow();
}

/** Merge `patch` into one node's config. */
export function simUpdateNodeConfig(nodeId: string, patch: Partial<NodeConfig>): void {
  engine.updateNodeConfig(nodeId, patch);
  publishNow();
}

/** Inject a chaos failure onto one node. */
export function simInjectFailure(nodeId: string, kind: FailureKind, opts: FailureOpts): void {
  engine.injectFailure(nodeId, kind, opts);
  publishNow();
}

/** Clear whatever failure is active on one node. */
export function simClearFailure(nodeId: string): void {
  engine.clearFailure(nodeId);
  publishNow();
}

/** Reset the live engine to a fresh run of its current topology. */
export function simReset(): void {
  engine.reset();
  publishNow();
}

/**
 * Advance the engine by one fixed step and publish immediately. Field-for-
 * field port of upstream's `handleStep` contract: the caller is responsible
 * for having already paused (`setRunning(false)`) so a step never races the
 * loop -- this function does not pause on its own, matching the old Rust
 * command's documented contract so call sites didn't need to change.
 */
export function simStep(deltaMs = 100): void {
  engine.advance(deltaMs);
  publishNow();
}

/** Pull the current snapshot on demand, bypassing the reactive store. */
export function simGetSnapshot(): SimSnapshot {
  return engine.snapshot();
}
