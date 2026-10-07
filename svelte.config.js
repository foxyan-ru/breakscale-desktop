import adapter from '@sveltejs/adapter-static';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/**
 * adapter-static + a single prerendered shell: Tauri serves the built app
 * from disk with no Node/SvelteKit server at runtime, so there is nothing
 * for SSR or a dynamic adapter to talk to. `fallback` covers any route the
 * prerenderer did not visit; today the app has exactly one route.
 */
/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    adapter: adapter({
      pages: 'build',
      assets: 'build',
      fallback: 'index.html',
      precompress: false,
      strict: true,
    }),
  },
};

export default config;
