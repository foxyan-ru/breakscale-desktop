/**
 * Reacts to `settingsStore.theme` by calling `applyTheme` (from
 * `$lib/theme/apply-theme`), which only ever sets or clears the
 * `data-theme` attribute -- `app.css`'s `@media (prefers-color-scheme:
 * dark)` block (ported from `src/index.css`) does the actual system-theme
 * following from there with no JS involved, exactly as it does in the web
 * app. This store's own job is narrower: keep a `resolvedTheme` field
 * (`'light' | 'dark'`, never `'system'`) for UI that wants to *label* the
 * current choice, e.g. "Following system (currently dark)" -- the DOM
 * attribute does not need this, but a settings toggle showing the effective
 * theme does.
 */

import { applyTheme, resolveSystemTheme } from '../theme/apply-theme';
import { settingsStore } from './settings.svelte';

export const themeState = $state<{ resolvedTheme: 'light' | 'dark' }>({
  resolvedTheme: 'light',
});

/**
 * Start reacting to `settingsStore.theme` and to OS theme changes.
 *
 * Call from `onMount` only -- `window.matchMedia` does not exist during the
 * prerender build step. Call the returned cleanup function from `onDestroy`.
 */
export function startTheme(): () => void {
  const media = window.matchMedia('(prefers-color-scheme: dark)');

  const apply = () => {
    applyTheme(settingsStore.theme);
    themeState.resolvedTheme =
      settingsStore.theme === 'system' ? resolveSystemTheme() : settingsStore.theme;
  };

  media.addEventListener('change', apply);

  // $effect.root opens a reactive scope that is not tied to a component's
  // lifecycle, so it can be started and stopped explicitly from onMount /
  // onDestroy the same way the media listener above is.
  const stopEffect = $effect.root(() => {
    $effect(() => {
      // Reading settingsStore.theme is what makes this effect re-run
      // whenever the preference changes.
      void settingsStore.theme;
      apply();
    });
  });

  return () => {
    media.removeEventListener('change', apply);
    stopEffect();
  };
}
