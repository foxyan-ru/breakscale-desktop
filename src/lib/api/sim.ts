/**
 * Typed wrappers around the `sim_*` Tauri commands, plus the `sim://*` event
 * listeners.
 *
 * This is the ONLY module allowed to call `invoke()`/`listen()` for the
 * simulation domain -- every Svelte component and store goes through it.
 * Call-site object keys are camelCase; Tauri's IPC layer converts them to
 * the Rust commands' snake_case parameter names automatically.
 */

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { FailureKind, FailureOpts, NodeConfig, SimSnapshot, Topology } from '$lib/domain';

/** Create a fresh engine for `topology`, optionally seeded for determinism. */
export async function simNew(topology: Topology, seed?: number): Promise<void> {
  return invoke('sim_new', { topology, seed });
}

/** Swap the live engine's topology in place (same engine, new wiring). */
export async function simSetTopology(topology: Topology): Promise<void> {
  return invoke('sim_set_topology', { topology });
}

/** Merge `patch` into one node's config. */
export async function simUpdateNodeConfig(
  nodeId: string,
  patch: Partial<NodeConfig>,
): Promise<void> {
  return invoke('sim_update_node_config', { nodeId, patch });
}

/** Inject a chaos failure onto one node. */
export async function simInjectFailure(
  nodeId: string,
  kind: FailureKind,
  opts: FailureOpts,
): Promise<void> {
  return invoke('sim_inject_failure', { nodeId, kind, opts });
}

/** Clear whatever failure is active on one node. */
export async function simClearFailure(nodeId: string): Promise<void> {
  return invoke('sim_clear_failure', { nodeId });
}

/** Reset the live engine to a fresh run of its current topology. */
export async function simReset(): Promise<void> {
  return invoke('sim_reset');
}

/** Pause or resume the background tick thread. */
export async function simSetRunning(running: boolean): Promise<void> {
  return invoke('sim_set_running', { running });
}

/** Pull the current snapshot on demand (the push path is `onSnapshot` below). */
export async function simGetSnapshot(): Promise<SimSnapshot> {
  return invoke('sim_get_snapshot');
}

/**
 * Subscribe to `sim://snapshot`, emitted at 10Hz by the background tick
 * thread (see `src-tauri/src/state.rs`).
 *
 * Call this from `onMount` only (the Tauri event API does not exist during
 * the prerender build step), and call the returned unlisten function from
 * `onDestroy`.
 */
export function onSnapshot(cb: (snapshot: SimSnapshot) => void): Promise<UnlistenFn> {
  return listen<SimSnapshot>('sim://snapshot', (e) => cb(e.payload));
}

/**
 * Subscribe to `sim://tick-error`, emitted if the engine's lock is ever
 * found poisoned mid-tick.
 *
 * Call this from `onMount` only, and call the returned unlisten function
 * from `onDestroy`.
 */
export function onTickError(cb: (message: string) => void): Promise<UnlistenFn> {
  return listen<{ message: string }>('sim://tick-error', (e) => cb(e.payload.message));
}
