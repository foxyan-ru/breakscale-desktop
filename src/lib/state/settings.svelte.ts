/**
 * Svelte 5 rune store mirroring the web app's `Preferences`
 * (`src/content/preferences.ts`): per-person choices about how much
 * interface to show, plus the vendor and theme choice.
 *
 * Defaults match `DEFAULT_PREFERENCES` in that file exactly (tooltips OFF,
 * sparklines ON, snap-to-grid ON, minimap OFF, vendor 'generic', theme
 * 'system').
 */

import type { VendorId } from '$lib/domain';

/** What the reader picked. `system` defers to the OS -- see `theme.svelte.ts`. */
export type ThemeChoice = 'light' | 'dark' | 'system';

export const THEME_CHOICES: readonly ThemeChoice[] = ['light', 'dark', 'system'];

export interface Settings {
  /** Show the dotted underlines and hover explanations on metric terms. */
  tooltips: boolean;
  /** Draw the small trend line on each node. */
  sparklines: boolean;
  /** Snap node positions to the grid while dragging. */
  snapToGrid: boolean;
  /** Show the minimap over the canvas. */
  minimap: boolean;
  /** Name components after a cloud vendor's products, or 'generic'. */
  vendor: VendorId;
  theme: ThemeChoice;
}

function createSettingsStore() {
  return $state<Settings>({
    tooltips: false,
    sparklines: true,
    snapToGrid: true,
    minimap: false,
    vendor: 'generic',
    theme: 'system',
  });
}

export const settingsStore = createSettingsStore();

// TODO(integration): wire to a settings_load/settings_save Tauri command
// once added. For now this is in-memory only and resets on relaunch.

export function setSetting<K extends keyof Settings>(key: K, value: Settings[K]): void {
  settingsStore[key] = value;
}
