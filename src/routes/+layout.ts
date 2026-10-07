// Tauri serves a static build with no SvelteKit server at runtime: there is
// nothing to server-render against, and `@tauri-apps/api` is only present in
// the webview at runtime, not during the Node-based prerender step. Every
// module that calls `invoke()` or `listen()` must therefore defer to
// `onMount`, never call it from top-level module or component-init code.
export const prerender = true;
export const ssr = false;
