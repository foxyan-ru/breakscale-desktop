/**
 * Cloud vendor and cost-model shapes.
 *
 * Mirrors `src/content/vendors/types.ts` and `src/content/vendors/cost.ts`
 * from the web app. Vendor specs are published facts (a vCPU count, a
 * memory figure, a price, each citable); the mapping from a spec to the
 * engine's `capacity`/`serviceMs` knobs is a documented MODEL, never
 * presented as a measurement -- see those source files' own commentary, and
 * MIGRATION_PLAN.md §4. The arithmetic (`fleetSize`, `costDesign`,
 * `deriveFromSize`) is ported to Rust (`vendors::cost`, `vendors::derive`);
 * this file is types only.
 */

import type { NodeKind } from './sim-types';

export type VendorId = 'generic' | 'aws' | 'gcp' | 'azure';

/** A published hardware size. Every field here is quotable. */
export interface VendorSize {
  /** Exactly as the vendor writes it, e.g. `db.r6g.large`. */
  name: string;
  /**
   * Absent where the vendor does not publish one in readable text. A missing
   * count means "nothing honest to say", not zero.
   */
  vcpu?: number;
  /** In the vendor's OWN unit. AWS and Azure publish GiB, GCP publishes GB. */
  memory: number;
  memoryUnit: 'GiB' | 'GB';
  /**
   * Network as PUBLISHED, kept as text (e.g. "Up to 10 Gigabit"). Turning
   * this into a number would quietly convert a ceiling or an estimate into
   * a promise.
   */
  network?: string;
  maxIops?: number;
  maxConnections?: number;
  /** On-demand, in the region named by the vendor file. */
  pricePerHour?: number;
  /** ISO date the price was read. Prices change; a stale one should show it. */
  pricedOn?: string;
  /** Where every number above came from. */
  source: string;
}

/** What one component kind is called at this vendor, and how it can be sized. */
export interface VendorMapping {
  /** The product name a student would see in the console. */
  product: string;
  /**
   * Why this product and not the vendor's other one. Present only where the
   * choice is genuinely arguable.
   */
  note?: string;
  /** Sizes worth offering. Absent where the vendor does not expose sizing. */
  sizes?: VendorSize[];
}

export interface Vendor {
  id: VendorId;
  /** How the vendor writes its own name. */
  label: string;
  /** The region every price in this file is quoted in. */
  region?: string;
  kinds: Partial<Record<NodeKind, VendorMapping>>;
}

export interface NodeCost {
  nodeId: string;
  label: string;
  /** The size this price came from, so the reader can check it. */
  sizeName: string;
  fleet: number;
  perHour: number;
  perMonth: number;
}

export interface DesignCost {
  nodes: NodeCost[];
  perMonth: number;
  /** Components with no priced size, which the total therefore excludes. */
  unpriced: string[];
}
