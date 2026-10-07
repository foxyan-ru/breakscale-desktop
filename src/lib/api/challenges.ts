/**
 * Types, Tauri command wrappers and pure helpers for the guided-scenario
 * ("challenge") feature.
 *
 * Ported from `src/sim/challenge.ts` + `src/sim/challenges.ts` (types and
 * pure logic) and `src/components/Challenge.tsx` / `Challenges.tsx` (what
 * the UI needs). Those two `sim/*.ts` files were already ported to Rust by
 * another agent as `sim::challenge` / `sim::challenges` (MIGRATION_PLAN.md
 * §4), but there is no `challenges_*` (or `challenge_*`) command in the
 * fixed `api/*.ts` contract as of this pass -- `api/sim.ts`, `api/designs.ts`,
 * `api/backup.ts`, `api/sysdesign.ts`, `api/dialog.ts` and their barrel
 * `api/index.ts` are marked finished/read-only, and none of them touch
 * challenges.
 *
 * NOTE(integration): backend commands "challenges_list" and "challenge_start"
 * not yet implemented as of this port; add matching #[tauri::command]s in
 * commands/challenges.rs during integration, backed by `sim::challenges`
 * (the `CHALLENGES` list + `challengeById`) and `sim::presets` + `sim::challenge`'s
 * `applyLoad`. Intended shapes:
 *
 *   challenges_list() -> ChallengeSummary[]
 *     Every challenge's id/name/brief, for the picker grid (`Challenges.svelte`).
 *     Mirrors iterating the web app's bundled `CHALLENGES` constant directly;
 *     the Tauri app has no client-side copy of that data; the earlier
 *     "invent a Tauri command" caveat below applies here too.
 *
 *   challenge_start({ id: string }) -> ChallengeStartResult
 *     Looks up the challenge and its preset by id server-side (mirrors
 *     `challengeById` + `PRESETS.find` in `src/App.tsx`'s `handleStartChallenge`),
 *     applies `applyLoad(topology, challenge.loadRps)`, and returns the full
 *     `Challenge` (including hints/lesson, which the summary omits) plus the
 *     resulting `Topology`. Deliberately does NOT also push the topology into
 *     the live engine or `topologyStore` -- that stays the caller's job via
 *     the already-existing `simNew`/`setTopology` (`api/sim.ts`,
 *     `state/topology.svelte.ts`), so this command has one job (produce a
 *     starting point) instead of duplicating "load a design" logic that
 *     already exists elsewhere.
 *
 * `evaluateChallenge` below is deliberately NOT a Tauri command, even though
 * a `challenge_evaluate` round trip is a reasonable first guess: `evaluate()`
 * in `src/sim/challenge.ts` is pure arithmetic over a `Challenge` and a
 * `SimSnapshot`, and the frontend already holds both (the challenge from
 * `challengeStart`, the snapshot streamed by `sim://snapshot` at 10Hz via
 * `simulationStore`) -- adding IPC for it would mean an extra round trip ten
 * times a second for a computation with no simulation state behind it. Same
 * reasoning as `$lib/components/cost/cost.ts` keeping `costDesign` in
 * TypeScript instead of inventing `vendors_cost`.
 */

import { invoke } from '@tauri-apps/api/core';
import type { SimSnapshot, Topology } from '$lib/domain';

export type GoalMetric = 'p99' | 'errorRate' | 'p95';

/** One checkable condition. `max` reads as "no more than". */
export interface Goal {
  metric: GoalMetric;
  max: number;
}

/** How one goal came out, with the number that decided it. */
export interface GoalResult {
  goal: Goal;
  actual: number;
  met: boolean;
}

export interface ChallengeResult {
  passed: boolean;
  goals: GoalResult[];
}

/** A brief with a pass condition the engine can check. */
export interface Challenge {
  id: string;
  /** Shown as the heading. A short imperative: "Survive the spike". */
  name: string;
  /** The brief, in the words a client would use rather than in metrics. */
  brief: string;
  presetId: string;
  /** Offered load the design is judged at, in requests per second. */
  loadRps: number;
  goals: Goal[];
  /** Nudges, in order, from the smallest one that helps to the one that names the component. */
  hints: string[];
  /** Why the fix worked, shown only once the goals are met. */
  lesson: string;
}

/** Enough to render the picker grid, without the hints/lesson a card never shows. */
export interface ChallengeSummary {
  id: string;
  name: string;
  brief: string;
}

export interface ChallengeStartResult {
  challenge: Challenge;
  /** The preset topology with `applyLoad` already applied at `challenge.loadRps`. */
  topology: Topology;
}

/** List every challenge, for the picker. */
export async function challengesList(): Promise<ChallengeSummary[]> {
  return invoke('challenges_list');
}

/** Start one challenge: resolves its preset + load, does not touch the live engine. */
export async function challengeStart(id: string): Promise<ChallengeStartResult> {
  return invoke('challenge_start', { id });
}

/**
 * Read the metric a goal names off a snapshot.
 *
 * `errorRate` is stored as a fraction and stated in the brief as a
 * percentage, because "under 1%" is how the requirement is written down in
 * real life and "under 0.01" is not.
 */
function actualFor(metric: GoalMetric, snapshot: SimSnapshot): number {
  const s = snapshot.system;
  switch (metric) {
    case 'p99':
      return s.p99;
    case 'p95':
      return s.p95;
    case 'errorRate':
      return s.errorRate * 100;
  }
}

/**
 * Judge a run. Pure and client-side -- see this module's own doc comment
 * for why this is not a Tauri command.
 *
 * A dead system is a FAILURE, never a pass: when nothing completes there are
 * no latencies to take a percentile of, so p99 reads 0 and would satisfy any
 * "under 200ms" goal it was given. The errorRate goal catches this on its
 * own in every brief that has one, but a brief judged only on latency would
 * hand out a pass for a total outage, so a run with no goodput fails
 * outright, whatever the other numbers say.
 */
export function evaluateChallenge(challenge: Challenge, snapshot: SimSnapshot): ChallengeResult {
  const dead = snapshot.system.goodputRps <= 0;
  const goals = challenge.goals.map((goal) => {
    const actual = actualFor(goal.metric, snapshot);
    return { goal, actual, met: !dead && actual <= goal.max };
  });
  return { passed: !dead && goals.every((g) => g.met), goals };
}

/**
 * Fields a brief holds still: `serviceMs` is how long the work takes, and
 * setting it to 1ms would pass every brief instantly and teach nothing, since
 * no engineer gets to declare their database forty times faster by typing a
 * number. `serviceCv` (how uneven the work is) is the same kind of property
 * of the work rather than a decision. Everything else -- including things
 * that look like cheating, like raising capacity to something absurd -- stays
 * editable, because it is a legitimate answer with a legitimate cost.
 */
export const FIXED_DURING_CHALLENGE: readonly string[] = ['serviceMs', 'serviceCv'];

export function isFixedDuringChallenge(field: string): boolean {
  return FIXED_DURING_CHALLENGE.includes(field);
}
