/**
 * Domain barrel: re-exports every type/constant in `lib/domain/*` for
 * `import { ... } from '$lib/domain'`.
 */

export * from './sim-types';
export * from './annotations';
export * from './design-file';
export * from './saved-designs';
export * from './backup';
export * from './vendors';
export * from './system-design';
export * from './clipboard';

/**
 * The shape every Tauri command rejection serialises to.
 *
 * Mirrors `src-tauri/src/error.rs`'s `AppError` enum: it is
 * `#[serde(rename_all = "camelCase", tag = "kind", content = "message")]`,
 * so a Rust `AppError::NotFound("...")` crosses the IPC boundary as
 * `{ kind: "notFound", message: "..." }`. `api/index.ts`'s `isAppError`
 * narrows a caught `invoke()` rejection to this shape.
 */
export interface AppErrorPayload {
  kind: 'validation' | 'notFound' | 'io' | 'serialization' | 'state';
  message: string;
}
