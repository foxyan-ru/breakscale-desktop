/**
 * System-design document shapes: high-level architecture plus low-level
 * design, seeded from a `Topology` and editable from there.
 *
 * This module is NEW -- it has no web-app source to port from (see
 * MIGRATION_PLAN.md §8). It mirrors `src-tauri/src/sysdesign/model.rs`
 * field-for-field; that Rust file is the SOURCE OF TRUTH (its
 * `#[serde(rename_all = "camelCase")]` is what makes this shape the actual
 * wire format) -- re-synced against it during integration after the two
 * were found to have drifted (built in parallel, each refined
 * independently of the other; see MIGRATION_PLAN.md's integration notes).
 */

import type { Topology } from './sim-types';
import type { VendorId } from './vendors';

export interface SystemDesignDoc {
  /** Unique id for this document, independent of any saved-design id. */
  id: string;
  name: string;
  schemaVersion: number;
  createdAt: string;
  updatedAt: string;
  topology: Topology;
  architecture: HighLevelArchitecture;
  lowLevel: LowLevelDesign;
}

export interface HighLevelArchitecture {
  summary: string;
  components: ComponentNote[];
  dataFlows: DataFlowNote[];
  externalDependencies: ExternalDependency[];
  qualityAttributes: QualityAttributes;
}

export interface ComponentNote {
  nodeId: string;
  technology: string;
  rationale: string;
  owner: string | null;
}

export interface DataFlowNote {
  edgeId: string;
  protocol: string;
  dataClassification: string;
  description: string;
}

export interface ExternalDependency {
  id: string;
  name: string;
  description: string;
  owner: string | null;
}

export interface QualityAttributes {
  scalability: string;
  reliability: string;
  security: string;
  observability: string;
  cost: string;
}

export interface LowLevelDesign {
  apis: ApiContract[];
  entities: EntityModel[];
  sequences: SequenceFlow[];
  stateMachines: StateMachine[];
  deployment: DeploymentConfig;
}

export interface ApiContract {
  id: string;
  method: string;
  path: string;
  description: string;
  requestSchema: string;
  responseSchema: string;
  errors: ApiError[];
}

export interface ApiError {
  statusCode: number;
  code: string;
  description: string;
}

export interface EntityModel {
  id: string;
  name: string;
  description: string;
  fields: EntityField[];
  relationships: EntityRelationship[];
}

export interface EntityField {
  name: string;
  fieldType: string;
  required: boolean;
  description: string;
}

export interface EntityRelationship {
  toEntity: string;
  kind: string;
  description: string;
}

export interface SequenceFlow {
  id: string;
  name: string;
  steps: SequenceStep[];
}

export interface SequenceStep {
  from: string;
  to: string;
  action: string;
  note: string;
}

export interface StateMachine {
  id: string;
  name: string;
  states: string[];
  transitions: StateTransition[];
}

export interface StateTransition {
  from: string;
  to: string;
  trigger: string;
  description: string;
}

export interface DeploymentConfig {
  environment: string;
  vendor: VendorId | null;
  region: string | null;
  resources: DeploymentResource[];
  notes: string;
}

export interface DeploymentResource {
  nodeId: string;
  sizeName: string | null;
  notes: string;
}

export type ExportFormat = 'json' | 'yaml' | 'markdown' | 'terraform';

export interface ValidationIssue {
  field: string;
  message: string;
}
