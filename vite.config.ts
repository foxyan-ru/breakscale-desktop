import { sveltekit } from '@sveltejs/kit/vite';
import { defaultClientConditions, defineConfig } from 'vite';

// Set by `tauri dev` when testing against a physical device / different host.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [sveltekit()],

  // WHY: @sveltejs/vite-plugin-svelte@4's peer range is Vite 5, where
  // `resolve.conditions` entries were ADDED to Vite's built-in defaults. In
  // Vite 6 a configured `resolve.conditions` REPLACES the defaults instead
  // (see the Vite 6 migration guide: "if you previously specified ['custom']
  // ... you need to specify ['custom', ...defaultClientConditions] instead").
  // The plugin still sets `resolve.conditions: ['svelte']` unconditionally
  // (its own Vite-6 fix landed in v5, which this repo isn't on -- bumping it
  // is a dependency-version change, not something to do without asking), so
  // every client build silently lost the `browser` condition. Svelte's
  // package export map then resolved `svelte` to its SERVER entry even in
  // the browser bundle: `onMount` became a no-op, `SvelteSet`/`SvelteMap`
  // (topology.svelte.ts's `selectedIds`, Tooltip.svelte's internal map)
  // stopped being reactive, and `tick`/`flushSync`/`getContext` no-op'd too.
  // That's the real root cause behind three earlier "onMount doesn't run"
  // workaround commits (eac998b, 57c1871, 2f04500) -- this restores the
  // default client conditions ahead of the plugin's own (concatenated, not
  // replaced) so client builds resolve `svelte`/`svelte/...` correctly again.
  resolve: {
    conditions: [...defaultClientConditions],
  },

  // Tauri prints its own build output; a second clear from Vite hides it.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: 'ws',
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // Rebuilding the frontend on a Rust file edit is wasted work; Cargo
      // watches src-tauri on its own.
      ignored: ['**/src-tauri/**'],
    },
  },
}));
