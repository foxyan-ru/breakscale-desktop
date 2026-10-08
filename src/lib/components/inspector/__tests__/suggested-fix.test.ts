import { render } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Inspector from '../Inspector.svelte';
import { HAS_THROUGHPUT_CEILING } from '../field-schema';
import { suggestionFor } from '$lib/content/suggestions';
import { topologyStore, select, clearSelection } from '$lib/state/topology.svelte';
import { simulationStore } from '$lib/state/simulation.svelte';
import { makeNode } from '$lib/components/canvas/geometry';
import type { NodeKind, NodeStats, SimSnapshot } from '$lib/domain';

/**
 * The suggestion is gated on headroom, and headroom is derived inside
 * Inspector.svelte from live stats (`headroomFor`, field-schema.ts). A unit
 * test on `suggestionFor` alone (`$lib/content/__tests__/suggestions.test.ts`)
 * can only prove the strings; it cannot prove a node ever shows one, nor
 * that a healthy node stays quiet. These render the real panel and read the
 * DOM, same spirit as upstream's `Inspector.suggestion.test.tsx`
 * (`eb163673`, PR #71) -- adapted to `@testing-library/svelte` instead of a
 * hand-rolled React root, since this port already has that harness.
 *
 * `HAS_THROUGHPUT_CEILING` is read from field-schema.ts rather than copied
 * as a literal list a second time, for the same reason the upstream PR's
 * last commit drove its own test off `KIND_NAME` instead of a hardcoded
 * list: a copied list stays green if a new ceiling kind ships with no
 * suggestion wired up; reading the real set does not.
 */
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve(null)) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));

const ALL_CEILING_KINDS = [...HAS_THROUGHPUT_CEILING] as NodeKind[];

/** geometry.ts's `defaultNodeConfig` gives every kind capacity=10,
 * serviceMs=20 -- a 500 req/s ceiling at the default single instance.
 * `OVERLOADED` clears every kind's ceiling comfortably; `UNDER_CEILING`
 * sits comfortably below all of them. */
const OVERLOADED = 2_000_000;
const UNDER_CEILING = 1;

/** Only the fields `NodeStats` requires; every reading the panel cares about
 * here (`arrivalRate`) is set explicitly, the rest are inert placeholders. */
function statsWith(arrivalRate: number): NodeStats {
  return {
    inFlight: 1,
    queued: 0,
    throughput: arrivalRate,
    arrivalRate,
    utilization: 0.99,
    p50: 10,
    p95: 20,
    p99: 30,
    errorRate: 0,
    shedRate: 0,
    timeoutRate: 0,
    hitRate: 0.8,
    totalCompleted: 0,
    totalFailed: 0,
    queueLimit: 256,
    staleReadRate: 0,
    maxShardUtilization: 0,
    minShardUtilization: 0,
    shardUtilization: [],
  };
}

/** Selects a fresh node of `kind` and seeds `simulationStore.snapshot` with
 * one node's stats at `arrivalRate` -- `null` leaves the snapshot absent
 * entirely (engine not started yet), the same loading state the vitals row
 * also gates on. Only the keys Inspector.svelte actually reads are real;
 * the rest are cast through, matching this repo's existing fixture
 * convention (see `ipc-contract.test.ts`'s `as unknown as` fixtures). */
function selectWithStats(kind: NodeKind, arrivalRate: number | null): void {
  const node = makeNode(kind, 0, 0);
  topologyStore.topology.nodes.push(node);
  select(node.id, null);
  simulationStore.snapshot =
    arrivalRate === null
      ? null
      : ({ nodes: { [node.id]: statsWith(arrivalRate) } } as unknown as SimSnapshot);
}

function suggestionText(container: Element): string | null {
  return container.querySelector('.ins-suggestion')?.textContent?.trim() ?? null;
}

describe('the suggested fix in the inspector', () => {
  afterEach(() => {
    clearSelection();
    topologyStore.topology.nodes.length = 0;
    simulationStore.snapshot = null;
  });

  it.each(ALL_CEILING_KINDS)('gives %s the text its kind defines once it cannot keep up', (kind) => {
    selectWithStats(kind, OVERLOADED);
    const { container } = render(Inspector);
    expect(suggestionText(container)).toBe(suggestionFor(kind));
  });

  it.each(ALL_CEILING_KINDS)('stays quiet for %s while it has headroom', (kind) => {
    selectWithStats(kind, UNDER_CEILING);
    const { container } = render(Inspector);
    expect(suggestionText(container)).toBeNull();
  });

  it('stays quiet when nothing is arriving, rather than reading zero as overloaded', () => {
    // Headroom is null at zero arrivals -- treating that as "below 1.0x"
    // would suggest a fix for a node that is merely idle.
    selectWithStats('service', 0);
    const { container } = render(Inspector);
    expect(suggestionText(container)).toBeNull();
  });

  it('offers nothing for a kind with no throughput ceiling, no matter the load', () => {
    selectWithStats('client', OVERLOADED);
    const { container } = render(Inspector);
    expect(suggestionText(container)).toBeNull();
  });

  it('shows nothing before the engine has started (no snapshot at all)', () => {
    selectWithStats('service', null);
    const { container } = render(Inspector);
    expect(suggestionText(container)).toBeNull();
  });

  it('does not tell an overloaded database to get bigger', () => {
    // The one suggestion upstream's issue thread (#23) singles out as a
    // trap: a plausible-sounding fix that makes nothing better.
    selectWithStats('db', OVERLOADED);
    const { container } = render(Inspector);
    expect(suggestionText(container)).not.toMatch(/add (more )?(capacity|instances)/i);
  });
});
