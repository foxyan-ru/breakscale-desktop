import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  onSnapshot,
  onTickError,
  simGetSnapshot,
  simNew,
  simReset,
  simSetRunning,
  simSetTopology,
  simUpdateNodeConfig,
} from '../sim';
import { presetLoad, presetsList } from '../presets';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

const invokeMock = vi.mocked(invoke);
const listenMock = vi.mocked(listen);

/**
 * The wire contract between the frontend and the Rust commands: every
 * `sim_*`/`preset_*` wrapper must invoke the command under exactly the
 * name `lib.rs` registers (`#[tauri::command]` renames to snake_case, and
 * the handler list only matches if both sides agree) with exactly the
 * payload keys the Rust parameters deserialize from. A rename or a
 * key-shape drift here is how every engine call would start rejecting at
 * runtime while TypeScript still compiles.
 */
describe('sim command wrappers', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    listenMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
  });

  it('simNew invokes sim_new with the topology payload', async () => {
    const topology = { nodes: [], edges: [] };

    await simNew(topology);

    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith('sim_new', { topology, seed: undefined });
  });

  it('simNew forwards an explicit seed for deterministic replays', async () => {
    const topology = { nodes: [], edges: [] };

    await simNew(topology, 42);

    expect(invokeMock).toHaveBeenCalledWith('sim_new', { topology, seed: 42 });
  });

  it('simSetTopology invokes sim_set_topology', async () => {
    const topology = { nodes: [], edges: [] };

    await simSetTopology(topology);

    expect(invokeMock).toHaveBeenCalledWith('sim_set_topology', { topology });
  });

  it('simUpdateNodeConfig sends the camelCased nodeId key', async () => {
    await simUpdateNodeConfig('node-1', { capacity: 20 });

    expect(invokeMock).toHaveBeenCalledWith('sim_update_node_config', {
      nodeId: 'node-1',
      patch: { capacity: 20 },
    });
  });

  it('simReset and simGetSnapshot invoke their commands bare', async () => {
    await simReset();
    await simGetSnapshot();

    expect(invokeMock).toHaveBeenNthCalledWith(1, 'sim_reset');
    expect(invokeMock).toHaveBeenNthCalledWith(2, 'sim_get_snapshot');
  });

  it('simSetRunning invokes sim_set_running with the flag', async () => {
    await simSetRunning(true);

    expect(invokeMock).toHaveBeenCalledWith('sim_set_running', { running: true });
  });

  it('propagates command rejections so callers can surface them', async () => {
    const rejection = { kind: 'state', message: 'No design is loaded yet.' };
    invokeMock.mockRejectedValue(rejection);

    await expect(simReset()).rejects.toEqual(rejection);
  });
});

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
});

describe('snapshot event subscriptions', () => {
  beforeEach(() => {
    listenMock.mockReset();
    listenMock.mockResolvedValue(() => {});
  });

  it('onSnapshot subscribes to the sim://snapshot channel', async () => {
    const unlisten = await onSnapshot(() => {});

    expect(listenMock).toHaveBeenCalledWith('sim://snapshot', expect.any(Function));
    expect(typeof unlisten).toBe('function');
  });

  it('onTickError subscribes to the sim://tick-error channel', async () => {
    const unlisten = await onTickError(() => {});

    expect(listenMock).toHaveBeenCalledWith('sim://tick-error', expect.any(Function));
    expect(typeof unlisten).toBe('function');
  });
});
