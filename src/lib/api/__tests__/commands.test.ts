import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { presetLoad, presetsList } from '../presets';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);

/**
 * The wire contract between the frontend and the Rust commands: every
 * `preset_*` wrapper must invoke the command under exactly the name
 * `lib.rs` registers (`#[tauri::command]` renames to snake_case, and the
 * handler list only matches if both sides agree) with exactly the payload
 * keys the Rust parameters deserialize from. A rename or a key-shape drift
 * here is how a call would start rejecting at runtime while TypeScript
 * still compiles. (The `sim_*` wrappers and their `sim://*` listeners are
 * gone: the engine runs in-process now, `$lib/sim/`.)
 */
describe('preset command wrappers', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
  });

  it('presetsList invokes presets_list with no payload', async () => {
    await presetsList();

    expect(invokeMock).toHaveBeenCalledWith('presets_list');
  });

  it('presetLoad invokes preset_load with the id', async () => {
    await presetLoad('single-server');

    expect(invokeMock).toHaveBeenCalledWith('preset_load', { id: 'single-server' });
  });

  // WHY: carried over from the removed sim-wrapper block -- the wrappers
  // must hand the Rust `{ kind, message }` rejection straight to the caller
  // (which narrows it with `isAppError`), never swallow or rewrap it.
  it('propagates command rejections so callers can surface them', async () => {
    const rejection = { kind: 'notFound', message: 'That example does not exist.' };
    invokeMock.mockRejectedValue(rejection);

    await expect(presetLoad('no-such-preset')).rejects.toEqual(rejection);
  });
});
