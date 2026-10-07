/**
 * Svelte 5 rune store for the current `Topology` plus canvas selection.
 *
 * Every mutation function updates the local `$state` optimistically, then
 * calls the matching `api/sim.ts` function to keep the Rust engine in sync.
 * The sync call is fire-and-forget: failures are routed into
 * `uiStore.errors` via `pushError` rather than left as an unhandled promise
 * rejection, and the local state is NOT rolled back on failure (the engine
 * call is a best-effort mirror; `simSetTopology`/`simNew` on the next full
 * sync is what actually reconciles it).
 *
 * SELECTION MODEL. `selectedIds` is the one selection model for the whole
 * canvas: node ids, edge ids and annotation ids all live together in one
 * set, exactly like the web app's `ReadonlySet<string> selectedIds`
 * (`src/components/Canvas.tsx`). This REPLACES the earlier
 * `selectedNodeId: string | null` / `selectedEdgeId: string | null` pair
 * (one node XOR one edge, no multi-select, no annotations) that this file
 * shipped with before Canvas.svelte grew multi-select, marquee-select,
 * Ctrl+A and clipboard copy/paste. `selectedNodeId`/`selectedEdgeId` are
 * kept below as derived single-selection READS -- not writable fields
 * anymore -- for the handful of consumers (`Inspector.svelte`,
 * `routes/+page.svelte`) that only ever expect ONE thing selected at a
 * time; each reads null the instant more than one id is selected, or the
 * one selected id is not that kind, matching the original's own
 * single-selection derivation (`Canvas.tsx`: `if (selectedIds.size !== 1)
 * return null`).
 */

import { SvelteSet } from 'svelte/reactivity';
import { simSetTopology, simUpdateNodeConfig } from '$lib/api/sim';
import { isAppError } from '$lib/api';
import { pushError } from './ui.svelte';
import type { NodeConfig, SimEdge, SimNode, Topology } from '$lib/domain';

interface TopologyState {
  topology: Topology;
  selectedIds: SvelteSet<string>;
  /** Derived: the one selected node id, or null. See file header comment. */
  readonly selectedNodeId: string | null;
  /** Derived: the one selected edge id, or null. See file header comment. */
  readonly selectedEdgeId: string | null;
}

function createTopologyStore() {
  return $state<TopologyState>({
    topology: { nodes: [], edges: [] },
    selectedIds: new SvelteSet<string>(),
    get selectedNodeId() {
      if (this.selectedIds.size !== 1) return null;
      const [id] = this.selectedIds;
      return this.topology.nodes.some((n: SimNode) => n.id === id) ? id : null;
    },
    get selectedEdgeId() {
      if (this.selectedIds.size !== 1) return null;
      const [id] = this.selectedIds;
      return this.topology.edges.some((e: SimEdge) => e.id === id) ? id : null;
    },
  });
}

export const topologyStore = createTopologyStore();

function reportSyncFailure(action: string, e: unknown): void {
  const message = isAppError(e) ? e.message : e instanceof Error ? e.message : String(e);
  pushError(`${action} failed to reach the simulation engine: ${message}`);
}

/** Replace the whole topology (e.g. loading a design) and push it to Rust. */
export function setTopology(topology: Topology): void {
  topologyStore.topology = topology;
  topologyStore.selectedIds.clear();
  simSetTopology(topology).catch((e) => reportSyncFailure('Loading the design', e));
}

/** Add a node to the topology. */
export function addNode(node: SimNode): void {
  const next: Topology = {
    ...topologyStore.topology,
    nodes: [...topologyStore.topology.nodes, node],
  };
  topologyStore.topology = next;
  simSetTopology(next).catch((e) => reportSyncFailure('Adding the component', e));
}

/** Merge `patch` into one node's config. */
export function updateNodeConfig(nodeId: string, patch: Partial<NodeConfig>): void {
  topologyStore.topology = {
    ...topologyStore.topology,
    nodes: topologyStore.topology.nodes.map((n) =>
      n.id === nodeId ? { ...n, config: { ...n.config, ...patch } } : n,
    ),
  };
  simUpdateNodeConfig(nodeId, patch).catch((e) =>
    reportSyncFailure('Updating the component', e),
  );
}

/**
 * Rename a node's label.
 *
 * There is no dedicated `sim_rename_node` command -- a label is
 * presentation data the engine carries but never reads -- so, like
 * `moveNode` below, this pushes the whole topology. The canvas's
 * double-click rename editor is the one caller; it already trims and
 * rejects an empty/unchanged label before calling this (see
 * `Canvas.svelte`'s `commitNodeRenameEditor`), so no validation happens
 * here either.
 */
export function renameNode(nodeId: string, label: string): void {
  const next: Topology = {
    ...topologyStore.topology,
    nodes: topologyStore.topology.nodes.map((n) => (n.id === nodeId ? { ...n, label } : n)),
  };
  topologyStore.topology = next;
  simSetTopology(next).catch((e) => reportSyncFailure('Renaming the component', e));
}

/**
 * Move a node to `(x, y)`.
 *
 * There is no dedicated `sim_move_node` command -- position is presentation
 * data the engine carries but never reads -- so this pushes the whole
 * topology like every other structural change. Callers driving a drag
 * gesture should call this on drag-end (or throttled), not once per pointer
 * frame, to avoid flooding IPC.
 */
export function moveNode(nodeId: string, x: number, y: number): void {
  const next: Topology = {
    ...topologyStore.topology,
    nodes: topologyStore.topology.nodes.map((n) => (n.id === nodeId ? { ...n, x, y } : n)),
  };
  topologyStore.topology = next;
  simSetTopology(next).catch((e) => reportSyncFailure('Moving the component', e));
}

/** Remove a node and every edge touching it. */
export function removeNode(nodeId: string): void {
  const topology = topologyStore.topology;
  const next: Topology = {
    ...topology,
    nodes: topology.nodes.filter((n) => n.id !== nodeId),
    edges: topology.edges.filter((e) => e.from !== nodeId && e.to !== nodeId),
  };
  topologyStore.topology = next;
  topologyStore.selectedIds.delete(nodeId);
  for (const e of topology.edges) {
    if (e.from === nodeId || e.to === nodeId) topologyStore.selectedIds.delete(e.id);
  }
  simSetTopology(next).catch((e) => reportSyncFailure('Removing the component', e));
}

/** Wire a new edge. */
export function addEdge(edge: SimEdge): void {
  const next: Topology = {
    ...topologyStore.topology,
    edges: [...topologyStore.topology.edges, edge],
  };
  topologyStore.topology = next;
  simSetTopology(next).catch((e) => reportSyncFailure('Adding the connection', e));
}

/** Remove an edge. */
export function removeEdge(edgeId: string): void {
  const topology = topologyStore.topology;
  const next: Topology = {
    ...topology,
    edges: topology.edges.filter((e) => e.id !== edgeId),
  };
  topologyStore.topology = next;
  topologyStore.selectedIds.delete(edgeId);
  simSetTopology(next).catch((e) => reportSyncFailure('Removing the connection', e));
}

/**
 * Delete every selected node AND edge in one topology edit. An edge
 * touching a deleted node is dropped too even when it was not itself in
 * `edgeIds` -- an orphaned edge left pointing at a node that no longer
 * exists would be a dangling reference the engine could not route.
 * Field-for-field port of the web app's `handleDeleteSelection`
 * (`src/App.tsx`), minus the annotation bucket: annotations are not this
 * store's concern (see `Canvas.svelte`'s own `commitAnnotations`), so a
 * caller deleting a mixed selection calls this for the node/edge ids and
 * handles any selected annotation ids itself.
 */
export function removeSelection(nodeIds: readonly string[], edgeIds: readonly string[]): void {
  if (nodeIds.length === 0 && edgeIds.length === 0) return;
  const dropNodes = new Set(nodeIds);
  const dropEdges = new Set(edgeIds);
  const topology = topologyStore.topology;
  const next: Topology = {
    ...topology,
    nodes: topology.nodes.filter((n) => !dropNodes.has(n.id)),
    edges: topology.edges.filter(
      (e) => !dropEdges.has(e.id) && !dropNodes.has(e.from) && !dropNodes.has(e.to),
    ),
  };
  topologyStore.topology = next;
  for (const id of nodeIds) topologyStore.selectedIds.delete(id);
  for (const id of edgeIds) topologyStore.selectedIds.delete(id);
  simSetTopology(next).catch((e) => reportSyncFailure('Removing the selection', e));
}

/**
 * Set the canvas selection. Back-compat single-selection helper: selects at
 * most one node XOR one edge, replacing whatever else was selected (nodes,
 * edges or annotations alike). Pass null for both to clear the selection
 * entirely. New call sites wanting additive (shift/ctrl) or multi-id
 * selection should use `selectOne`/`setSelection`/`selectAll` below instead.
 */
export function select(nodeId: string | null, edgeId: string | null): void {
  topologyStore.selectedIds.clear();
  if (nodeId) topologyStore.selectedIds.add(nodeId);
  else if (edgeId) topologyStore.selectedIds.add(edgeId);
}

/**
 * Click selection semantics: a plain click REPLACES the selection with just
 * `id`; an additive (shift/ctrl) click TOGGLES `id` within whatever is
 * already selected. Field-for-field port of the web app's `selectOne`
 * (`src/components/Canvas.tsx` ~3649). `id` may be a node, edge or
 * annotation id -- the set does not distinguish.
 */
export function selectOne(id: string, additive: boolean): void {
  if (!additive) {
    topologyStore.selectedIds.clear();
    topologyStore.selectedIds.add(id);
    return;
  }
  if (topologyStore.selectedIds.has(id)) topologyStore.selectedIds.delete(id);
  else topologyStore.selectedIds.add(id);
}

/** Replace the whole selection outright (a marquee's release, mainly). */
export function setSelection(ids: Iterable<string>): void {
  topologyStore.selectedIds.clear();
  for (const id of ids) topologyStore.selectedIds.add(id);
}

/** Select every node, edge and annotation id in the current topology (Ctrl+A). */
export function selectAll(): void {
  const next = new Set<string>();
  for (const n of topologyStore.topology.nodes) next.add(n.id);
  for (const e of topologyStore.topology.edges) next.add(e.id);
  for (const a of topologyStore.topology.annotations ?? []) next.add(a.id);
  setSelection(next);
}

/** Clear the canvas selection entirely. */
export function clearSelection(): void {
  topologyStore.selectedIds.clear();
}
