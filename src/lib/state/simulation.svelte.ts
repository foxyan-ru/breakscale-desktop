/**
 * Svelte 5 rune store for live simulation state: the latest snapshot pushed
 * by the Rust engine, whether the tick thread is running, and the last
 * tick-error message (if any).
 */

import { onSnapshot, onTickError, simSetRunning } from '$lib/api/sim';
import { pushError } from './ui.svelte';
import type { SimSnapshot } from '$lib/domain';

interface SimulationState {
  snapshot: SimSnapshot | null;
  running: boolean;
  tickError: string | null;
}

function createSimulationStore() {
  const state = $state<SimulationState>({
    snapshot: null,
    running: false,
    tickError: null,
  });
  return state;
}

export const simulationStore = createSimulationStore();

/**
 * Start listening for `sim://snapshot` and `sim://tick-error` events and
 * keep `simulationStore` in sync with them.
 *
 * Call this from the root layout's `onMount` only -- the underlying Tauri
 * event API does not exist during the prerender build step. Call the
 * returned cleanup function from `onDestroy`.
 */
export async function startListening(): Promise<() => void> {
  const unlistenSnapshot = await onSnapshot((snapshot) => {
    simulationStore.snapshot = snapshot;
  });
  const unlistenTickError = await onTickError((message) => {
    simulationStore.tickError = message;
  });

  return () => {
    unlistenSnapshot();
    unlistenTickError();
  };
}

/**
 * Pause or resume the background tick thread, optimistically updating
 * `simulationStore.running` and keeping the Rust engine in sync.
 */
export function setRunning(running: boolean): void {
  simulationStore.running = running;
  simSetRunning(running).catch((e) => {
    simulationStore.running = !running;
    pushError(`Could not ${running ? 'resume' : 'pause'} the simulation: ${describe(e)}`);
  });
}

function describe(e: unknown): string {
  if (e && typeof e === 'object' && 'message' in e && typeof e.message === 'string') {
    return e.message;
  }
  return e instanceof Error ? e.message : String(e);
}
