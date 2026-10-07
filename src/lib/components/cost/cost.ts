/**
 * What a design would cost to run, per month.
 *
 * Faithful client-side port of `src/content/vendors/cost.ts` from the web
 * app. `NodeCost`/`DesignCost`/`Vendor`/`VendorSize` are NOT redefined here
 * -- they already live in `$lib/domain` (`domain/vendors.ts`), ported
 * alongside the rest of the vendor shapes -- so this module imports them and
 * carries only the arithmetic.
 *
 * WHY THIS IS TYPESCRIPT AND NOT A TAURI COMMAND. MIGRATION_PLAN.md §4 lists
 * `vendors::cost` as the intended Rust home for this arithmetic, and it is
 * being ported there in parallel (see `src-tauri/src/vendors/cost.rs`).
 * But `api/*.ts` is fixed for this pass and has no `vendors_*` command yet
 * (see `$lib/api/vendors.ts`'s own NOTE(integration)), and costDesign is a
 * few lines of pure, stateless multiplication (rate * fleet * hours) with no
 * simulation state behind it and no side effects -- inventing a Tauri round
 * trip for it this late, on top of one for `vendors_get`, would add IPC
 * latency to a number `Cost.svelte` recomputes on every render for no
 * correctness benefit. Once a real `vendors_cost` (or equivalent) command
 * exists, this file can be deleted and `Cost.svelte` switched to call it
 * instead -- nothing outside this module and `Cost.svelte` depends on the
 * arithmetic living here.
 *
 * WHAT THIS IS HONEST ABOUT (carried over from the source file). Every
 * hourly rate comes from a vendor file and carries a source URL; the
 * arithmetic here is genuinely how compute bills work: rate * fleet size *
 * hours. What it is NOT is a bill -- real invoices carry storage, egress,
 * backup, support and reserved-instance discounts, none of which this
 * models, which is why `COST_NOTE` below is shown next to every total.
 */

import type { DesignCost, NodeConfig, NodeCost, NodeKind, SimNode, Vendor, VendorSize } from '$lib/domain';

/**
 * Hours in an average month: 730 (365 days / 12), not 720. Vendors bill per
 * hour and quote monthly figures on a 730-hour month, so using 30 days would
 * put every number here about 1.4 percent below the vendor's own calculator
 * for no reason anyone could see.
 */
export const HOURS_PER_MONTH = 730;

/**
 * How many of this thing are actually running.
 *
 * Reads what each kind really uses rather than assuming `instances`
 * everywhere: a shard's fleet is its partition count, a replica set's is its
 * replica count (the primary plus its replicas -- the primary is a box too).
 */
export function fleetSize(kind: NodeKind, config: NodeConfig): number {
  // Each field is optional on the type because only some kinds read it, so a
  // missing one means "this kind has no fleet" and falls back to one box
  // rather than to zero.
  const count = (v: number | undefined) =>
    typeof v === 'number' && Number.isFinite(v) ? Math.max(1, Math.floor(v)) : 1;

  if (kind === 'shard') return count(config.shardCount);
  if (kind === 'replica') return count(config.replicaCount) + 1;
  return count(config.instances);
}

/**
 * Cost a design, given the size chosen for each component.
 *
 * `sizeOf` returns the size a node is running on, or null where no size has
 * been chosen. A component with no size is NOT guessed at: it is listed as
 * unpriced, because inventing a rate for it would make the total look
 * complete while being wrong by an unknown amount.
 */
export function costDesign(
  nodes: readonly SimNode[],
  vendor: Vendor | null,
  sizeOf: (node: SimNode) => VendorSize | null,
): DesignCost {
  const out: NodeCost[] = [];
  const unpriced: string[] = [];
  if (!vendor) return { nodes: out, perMonth: 0, unpriced: [] };

  for (const node of nodes) {
    const size = sizeOf(node);
    if (!size || typeof size.pricePerHour !== 'number') {
      // A client is not a thing you rent, so it is not "unpriced" in the
      // sense of missing data. Only components the vendor actually sells
      // are reported as a gap in the total.
      if (node.kind !== 'client' && vendor.kinds[node.kind]) {
        unpriced.push(node.label);
      }
      continue;
    }
    const fleet = fleetSize(node.kind, node.config);
    const perHour = size.pricePerHour * fleet;
    out.push({
      nodeId: node.id,
      label: node.label,
      sizeName: size.name,
      fleet,
      perHour,
      perMonth: perHour * HOURS_PER_MONTH,
    });
  }

  out.sort((a, b) => b.perMonth - a.perMonth);
  return {
    nodes: out,
    perMonth: out.reduce((sum, n) => sum + n.perMonth, 0),
    unpriced,
  };
}

/** Money, in the way a reader scans a bill rather than reads a number. */
export function formatMoney(usd: number): string {
  if (usd >= 10_000) return `$${Math.round(usd).toLocaleString('en-US')}`;
  if (usd >= 100) return `$${usd.toFixed(0)}`;
  if (usd >= 1) return `$${usd.toFixed(2)}`;
  // Sub-dollar figures are real for a small instance; rounding to $0 would
  // tell a reader the thing is free.
  return `$${usd.toFixed(3)}`;
}

/** The caveat, in one place, so it cannot drift between two surfaces. */
export const COST_NOTE =
  'Compute only, at on-demand rates. A real bill also carries storage, network egress, backups and support, and discounts for commitment.';
