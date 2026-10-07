import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

// Set by `tauri dev` when testing against a physical device / different host.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [sveltekit()],

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
