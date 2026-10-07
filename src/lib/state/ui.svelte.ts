/**
 * Svelte 5 rune store for frontend-only UI state: panel visibility, the
 * active view, and a toast/error queue every other store routes into.
 *
 * `library`/`metrics` and the panel-size fields mirror the web app's
 * `LayoutPrefs`/`PANEL_LIMITS` (`src/App.tsx`) for parity with the panels
 * the UI-component agents are building; none of this is sent to Rust, so
 * field names beyond those two are this module's own judgement call.
 */

export type ActiveView = 'canvas' | 'glossary' | 'designs' | 'settings' | 'sysdesign';

/**
 * Size limits, in px, for the resizable panels. Mirrors `PANEL_LIMITS` in
 * `src/App.tsx`: the minimums are where a panel stops being able to show
 * its own content, the maximums stop a panel from taking the window.
 */
export const PANEL_LIMITS = {
  railW: { min: 180, max: 420, base: 224 },
  insW: { min: 260, max: 520, base: 320 },
  stripH: { min: 140, max: 420, base: 220 },
} as const;

export interface ToastError {
  id: string;
  message: string;
}

interface UiState {
  /** The left component rail. */
  library: boolean;
  /** The bottom charts strip. */
  metrics: boolean;
  /** Panel sizes in px, mirroring `LayoutPrefs`. */
  railW: number;
  insW: number;
  stripH: number;
  activeView: ActiveView;
  errors: ToastError[];
}

function createUiStore() {
  const state = $state<UiState>({
    library: true,
    metrics: false,
    railW: PANEL_LIMITS.railW.base,
    insW: PANEL_LIMITS.insW.base,
    stripH: PANEL_LIMITS.stripH.base,
    activeView: 'canvas',
    errors: [],
  });
  return state;
}

export const uiStore = createUiStore();

let errorSeq = 0;

/**
 * Queue an error/toast message. This is the shared sink every other store's
 * `.catch()` on a fire-and-forget `invoke()` call routes into, so a failed
 * background sync never becomes an unhandled promise rejection.
 */
export function pushError(message: string): void {
  errorSeq += 1;
  const id = `err${Date.now().toString(36)}${errorSeq.toString(36)}`;
  uiStore.errors = [...uiStore.errors, { id, message }];
}

export function dismissError(id: string): void {
  uiStore.errors = uiStore.errors.filter((e) => e.id !== id);
}
