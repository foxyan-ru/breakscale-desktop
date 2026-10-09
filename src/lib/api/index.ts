/**
 * API barrel: re-exports every typed Tauri command wrapper for
 * `import { ... } from '$lib/api'`, plus the `isAppError` helper for
 * narrowing a caught `invoke()` rejection.
 */

import type { AppErrorPayload } from '$lib/domain';

export * from './designs';
export * from './backup';
export * from './sysdesign';
export * from './dialog';

/**
 * Narrow a caught `invoke()` rejection to `AppErrorPayload`.
 *
 * `invoke()` rejects with whatever Tauri serialised the Rust `Err(AppError)`
 * as, which arrives at the catch site as `unknown`. Use this instead of
 * guessing at the shape:
 *
 * ```ts
 * try {
 *   await designsGet(id);
 * } catch (e) {
 *   if (isAppError(e)) {
 *     // e.kind, e.message
 *   }
 * }
 * ```
 */
export function isAppError(e: unknown): e is AppErrorPayload {
  return (
    typeof e === 'object' &&
    e !== null &&
    'kind' in e &&
    'message' in e &&
    typeof (e as Record<string, unknown>).kind === 'string' &&
    typeof (e as Record<string, unknown>).message === 'string'
  );
}
