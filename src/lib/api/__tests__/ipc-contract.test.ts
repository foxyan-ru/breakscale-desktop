import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import goldenJson from '../../../../contract/ipc-golden.json';

import type {
  ActiveFailure,
  ApiContract,
  ApiError,
  AppErrorPayload,
  BackupResult,
  ComponentNote,
  DataFlowNote,
  DeploymentConfig,
  DeploymentResource,
  DesignParseResult,
  EntityField,
  EntityModel,
  EntityRelationship,
  ExportFormat,
  ExternalDependency,
  FailureKind,
  FailureReason,
  HighLevelArchitecture,
  HistoryPoint,
  LowLevelDesign,
  NodeConfig,
  NodeStats,
  Note,
  QualityAttributes,
  RequestTrace,
  SaveResult,
  SavedDesign,
  SavedSummary,
  Section,
  SequenceFlow,
  SequenceStep,
  SimEdge,
  SimNode,
  SimSnapshot,
  StateMachine,
  StateTransition,
  SystemDesignDoc,
  SystemStats,
  Topology,
  TraceHop,
  ValidationIssue,
  Vendor,
  VendorId,
  VendorMapping,
  VendorSize,
} from '../../domain';
import type { GlossaryEntry } from '../../components/glossary/Glossary.svelte';
import type { PresetSummary } from '../../components/shell/Examples.svelte';
import {
  onSnapshot,
  onTickError,
  simClearFailure,
  simGetSnapshot,
  simInjectFailure,
  simNew,
  simReset,
  simSetRunning,
  simSetTopology,
  simStep,
  simUpdateNodeConfig,
} from '../sim';
import {
  designFileBuild,
  designFileParse,
  designFileRead,
  designFileWrite,
  designsDelete,
  designsGet,
  designsList,
  designsRename,
  designsSave,
} from '../designs';
import { backupRestoreFromPath, backupWrite } from '../backup';
import {
  sysdesignDeriveHighLevel,
  sysdesignExport,
  sysdesignLoad,
  sysdesignSave,
  sysdesignValidate,
} from '../sysdesign';
import { glossaryList } from '../glossary';
import { challengeStart, challengesList } from '../challenges';
import type { Challenge, ChallengeStartResult, ChallengeSummary, Goal } from '../challenges';
import { presetLoad, presetsList } from '../presets';
import type { Preset } from '../presets';
import { vendorsGet } from '../vendors';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

const invokeMock = vi.mocked(invoke);
const listenMock = vi.mocked(listen);

/**
 * The TypeScript half of the shared IPC contract suite -- sibling to
 * `src-tauri/src/contract_tests.rs` (12 tests), pinned against the SAME
 * golden file: `contract/ipc-golden.json`.
 *
 * WHY THIS FILE EXISTS. A command rename or a payload-key drift fails at
 * runtime with an opaque "command not found" / serde "missing field" long
 * after the change compiles. Rust cannot see the TS types and `tsc` cannot
 * see the Rust structs, so each side checks its half against the fixture:
 *
 * - COMPILE TIME (this file, checked by CI's `bun run check`): every
 *   fixture payload instance's key set equals the TS interface's key set,
 *   in both directions (see the `PinKeys` block below).
 * - RUNTIME (this file, checked by CI's `bun run test`): every wrapper
 *   invokes its command with EXACTLY the `commandArgs[cmd].all` keys
 *   (undefined-valued keys never cross the wire, so `required` keys are
 *   additionally checked non-undefined); the 20 payload key sets are
 *   shape-asserted; the `listen` channels equal `events`.
 *
 * KNOWN, DELIBERATE GAPS (mirrored from the Rust suite's header):
 * - The four settings/layout commands have NO TS caller yet (`settings.svelte.ts`
 *   carries `TODO(integration)`), so they are pinned here only through the
 *   fixture and the Rust signature test -- see `uninvokedCommands`.
 * - `Annotation` is `serde_json::Value` in Rust, so the note/section key
 *   sets are pinned here (compile + runtime) and only indirectly by Rust's
 *   `Topology` roundtrip.
 * - Fixture VALUES are illustrative samples (fixture `$comment`); this
 *   suite pins key sets and wire call shapes, never content strings.
 *
 * Reading order: fixture loading -> type-level pins -> value-level pins ->
 * runtime helpers -> inventories -> payload shapes -> wrapper invocations
 * -> events.
 */
const golden = JSON.parse(
  readFileSync(resolve(process.cwd(), 'contract/ipc-golden.json'), 'utf8'),
) as typeof goldenJson;

// ---------------------------------------------------------------------------
// Type-level pins: fixture instance key sets vs TS interface key sets.
//
// WHY a key-set equality and not a plain assignment. TypeScript WIDENS JSON
// module imports (`{ kind: "service" }` arrives as `kind: string`), so
// `const sample: SimNode = golden.payloads.topology.nodes[0]` cannot work
// for any interface with a string-literal union member, and a plain
// `expectTypeOf`-style assignability check would be blind to optional keys.
// `PinKeys<Fixture, Domain>` instead asks the two questions that actually
// break the wire:
//   1. a TS-required key missing from the fixture instance, and
//   2. a fixture key no TS interface declares.
// Both must be `never`, enforced through `Expect` at compile time (CI's
// `bun run check`), with zero runtime cost.
// ---------------------------------------------------------------------------

type Golden = typeof goldenJson;
type Payloads = Golden['payloads'];

/** First element type of a fixture array. */
type El<T> = T extends readonly (infer U)[] ? U : never;
/** Value union of a fixture keyed object (e.g. `simSnapshot.nodes`). */
type Values<T> = T[keyof T];

/** Keys whose property is required (`-?` strips optionality before Pick). */
type RequiredKeys<T> = { [K in keyof T]-?: {} extends Pick<T, K> ? never : K }[keyof T];

/**
 * Keys of a fixture instance that JSON could actually carry. TypeScript
 * normalises a heterogeneous JSON array union by tagging each member with
 * the OTHER member's keys as optional `undefined` (`label?: undefined` on
 * the note branch of `topology.annotations`) -- those markers are type
 * machinery, not keys the parsed value ever has (JSON has no `undefined`),
 * so they must not count as fixture keys in either direction of the pin.
 * For single-shape payloads this is exactly `keyof F`.
 */
type JsonKeys<F> = { [K in keyof F]-?: [F[K]] extends [undefined] ? never : K }[keyof F];

/**
 * Symmetric key-set difference between a fixture instance and a TS
 * interface; resolves to `never` only when both directions are empty.
 * Fixture-vs-domain order matters: `keyof` on a fixture object yields its
 * literal keys, `keyof` on the interface yields the declared ones.
 */
type PinKeys<F, D> =
  | Exclude<RequiredKeys<D>, JsonKeys<F>>
  | Exclude<JsonKeys<F>, keyof D>;
type Expect<T extends never> = T;

// -- payloads.simSnapshot ---------------------------------------------------
type _Pin_SimSnapshot = Expect<PinKeys<Payloads['simSnapshot'], SimSnapshot>>;
type _Pin_SystemStats = Expect<
  PinKeys<Payloads['simSnapshot']['system'], SystemStats>
>;
type _Pin_NodeStats = Expect<
  PinKeys<Values<Payloads['simSnapshot']['nodes']>, NodeStats>
>;
type _Pin_HistoryPoint = Expect<
  PinKeys<El<Payloads['simSnapshot']['history']>, HistoryPoint>
>;
type _Pin_ActiveFailure = Expect<
  PinKeys<El<Payloads['simSnapshot']['activeFailures']>, ActiveFailure>
>;
type _Pin_RequestTrace = Expect<
  PinKeys<Payloads['simSnapshot']['trace'], RequestTrace>
>;
type _Pin_TraceHop = Expect<
  PinKeys<El<Payloads['simSnapshot']['trace']['hops']>, TraceHop>
>;
type _Pin_FailureReasons = Expect<
  PinKeys<Payloads['simSnapshot']['failuresByReason'], Record<FailureReason, number>>
>;
// `edgeFlow`/`edgeState` are open `Record<string, ...>` maps: their TS key
// set is `string` by design, so no fixture key can be "missing" -- the Rust
// roundtrip (contract_tests.rs test 7) deep-pins the value keys instead.

// -- payloads.topology -----------------------------------------------------
type _Pin_Topology = Expect<PinKeys<Payloads['topology'], Topology>>;
type _Pin_SimNode = Expect<PinKeys<El<Payloads['topology']['nodes']>, SimNode>>;
type _Pin_SimEdge = Expect<PinKeys<El<Payloads['topology']['edges']>, SimEdge>>;
type _Pin_NodeConfig = Expect<
  PinKeys<El<Payloads['topology']['nodes']>['config'], NodeConfig>
>;

// -- annotations (the Rust-declared gap: `serde_json::Value` in Rust) ------
// The fixture array element is a union of the note and section shapes;
// discriminate by the properties only each member has (the `kind` literal
// is widened to `string` in JSON types, so it cannot discriminate here).
type JsonAnnotation = El<Payloads['topology']['annotations']>;
type _Pin_AnnotationNote = Expect<
  PinKeys<Extract<JsonAnnotation, { text: string }>, Note>
>;
type _Pin_AnnotationSection = Expect<
  PinKeys<Extract<JsonAnnotation, { label: string }>, Section>
>;

// -- payloads.systemDesignDoc ---------------------------------------------
type _Pin_SystemDesignDoc = Expect<
  PinKeys<Payloads['systemDesignDoc'], SystemDesignDoc>
>;
type _Pin_DocTopology = Expect<
  PinKeys<Payloads['systemDesignDoc']['topology'], Topology>
>;
type _Pin_Architecture = Expect<
  PinKeys<Payloads['systemDesignDoc']['architecture'], HighLevelArchitecture>
>;
type _Pin_ComponentNote = Expect<
  PinKeys<El<Payloads['systemDesignDoc']['architecture']['components']>, ComponentNote>
>;
type _Pin_DataFlowNote = Expect<
  PinKeys<El<Payloads['systemDesignDoc']['architecture']['dataFlows']>, DataFlowNote>
>;
type _Pin_ExternalDependency = Expect<
  PinKeys<
    El<Payloads['systemDesignDoc']['architecture']['externalDependencies']>,
    ExternalDependency
  >
>;
type _Pin_QualityAttributes = Expect<
  PinKeys<Payloads['systemDesignDoc']['architecture']['qualityAttributes'], QualityAttributes>
>;
type _Pin_LowLevel = Expect<
  PinKeys<Payloads['systemDesignDoc']['lowLevel'], LowLevelDesign>
>;
type _Pin_ApiContract = Expect<
  PinKeys<El<Payloads['systemDesignDoc']['lowLevel']['apis']>, ApiContract>
>;
type _Pin_ApiError = Expect<
  PinKeys<El<El<Payloads['systemDesignDoc']['lowLevel']['apis']>['errors']>, ApiError>
>;
type _Pin_EntityModel = Expect<
  PinKeys<El<Payloads['systemDesignDoc']['lowLevel']['entities']>, EntityModel>
>;
type _Pin_EntityField = Expect<
  PinKeys<El<El<Payloads['systemDesignDoc']['lowLevel']['entities']>['fields']>, EntityField>
>;
type _Pin_EntityRelationship = Expect<
  PinKeys<
    El<El<Payloads['systemDesignDoc']['lowLevel']['entities']>['relationships']>,
    EntityRelationship
  >
>;
type _Pin_SequenceFlow = Expect<
  PinKeys<El<Payloads['systemDesignDoc']['lowLevel']['sequences']>, SequenceFlow>
>;
type _Pin_SequenceStep = Expect<
  PinKeys<El<El<Payloads['systemDesignDoc']['lowLevel']['sequences']>['steps']>, SequenceStep>
>;
type _Pin_StateMachine = Expect<
  PinKeys<El<Payloads['systemDesignDoc']['lowLevel']['stateMachines']>, StateMachine>
>;
type _Pin_StateTransition = Expect<
  PinKeys<
    El<El<Payloads['systemDesignDoc']['lowLevel']['stateMachines']>['transitions']>,
    StateTransition
  >
>;
type _Pin_Deployment = Expect<
  PinKeys<Payloads['systemDesignDoc']['lowLevel']['deployment'], DeploymentConfig>
>;
type _Pin_DeploymentResource = Expect<
  PinKeys<
    El<Payloads['systemDesignDoc']['lowLevel']['deployment']['resources']>,
    DeploymentResource
  >
>;
type _Pin_ValidationIssue = Expect<
  PinKeys<El<Payloads['validationIssue']>, ValidationIssue>
>;

// -- result / summary payloads --------------------------------------------
type _Pin_SavedSummary = Expect<PinKeys<El<Payloads['savedSummary']>, SavedSummary>>;
type _Pin_SavedDesign = Expect<PinKeys<Payloads['savedDesign'], SavedDesign>>;
type _Pin_SavedTopology = Expect<
  PinKeys<Payloads['savedDesign']['topology'], Topology>
>;
type _Pin_SaveOk = Expect<
  PinKeys<Payloads['saveResultOk'], Extract<SaveResult, { ok: true }>>
>;
type _Pin_SaveErr = Expect<
  PinKeys<Payloads['saveResultErr'], Extract<SaveResult, { ok: false }>>
>;
type _Pin_ParseOk = Expect<
  PinKeys<Payloads['designParseResultOk'], Extract<DesignParseResult, { ok: true }>>
>;
type _Pin_ParseOkTopology = Expect<
  PinKeys<Payloads['designParseResultOk']['topology'], Topology>
>;
type _Pin_ParseErr = Expect<
  PinKeys<Payloads['designParseResultErr'], Extract<DesignParseResult, { ok: false }>>
>;
type _Pin_BackupOk = Expect<
  PinKeys<Payloads['backupResultOk'], Extract<BackupResult, { ok: true }>>
>;
type _Pin_BackupErr = Expect<
  PinKeys<Payloads['backupResultErr'], Extract<BackupResult, { ok: false }>>
>;

// -- content payloads ------------------------------------------------------
type _Pin_Vendor = Expect<PinKeys<Payloads['vendor'], Vendor>>;
type _Pin_VendorMapping = Expect<
  PinKeys<Payloads['vendor']['kinds']['cache'], VendorMapping>
>;
type _Pin_VendorSize = Expect<
  PinKeys<El<Payloads['vendor']['kinds']['cache']['sizes']>, VendorSize>
>;
type _Pin_GlossaryEntry = Expect<PinKeys<El<Payloads['glossaryEntry']>, GlossaryEntry>>;
type _Pin_PresetSummary = Expect<PinKeys<El<Payloads['presetSummary']>, PresetSummary>>;
type _Pin_Preset = Expect<PinKeys<Payloads['preset'], Preset>>;
type _Pin_PresetTopology = Expect<PinKeys<Payloads['preset']['topology'], Topology>>;
type _Pin_ChallengeSummary = Expect<
  PinKeys<El<Payloads['challengeSummary']>, ChallengeSummary>
>;
type _Pin_ChallengeStart = Expect<
  PinKeys<Payloads['challengeStart'], ChallengeStartResult>
>;
type _Pin_Challenge = Expect<PinKeys<Payloads['challengeStart']['challenge'], Challenge>>;
type _Pin_ChallengeTopology = Expect<
  PinKeys<Payloads['challengeStart']['topology'], Topology>
>;
type _Pin_Goal = Expect<
  PinKeys<El<Payloads['challengeStart']['challenge']['goals']>, Goal>
>;
type _Pin_AppErrorPayload = Expect<
  PinKeys<Payloads['appErrorPayload'], AppErrorPayload>
>;

// ---------------------------------------------------------------------------
// Value-level pins.
//
// WHY only these six. A plain assignment from a JSON import checks value
// TYPES too (not just keys) -- but it compiles only for interfaces with no
// string-literal members, because the import's string values are widened to
// `string`. Every interface above containing a union (`FailureKind`,
// `TrafficPattern`, `memoryUnit`, ...) is covered by the key pins plus the
// Rust roundtrip instead; assigning those here would fail `svelte-check`
// for a reason unrelated to drift.
// ---------------------------------------------------------------------------

const valuePinSystem: SystemStats = golden.payloads.simSnapshot.system;
const valuePinHistory: HistoryPoint = golden.payloads.simSnapshot.history[0];
const valuePinIssue: ValidationIssue = golden.payloads.validationIssue[0];
const valuePinSavedSummary: SavedSummary = golden.payloads.savedSummary[0];
const valuePinQuality: QualityAttributes =
  golden.payloads.systemDesignDoc.architecture.qualityAttributes;
const valuePinApiError: ApiError =
  golden.payloads.systemDesignDoc.lowLevel.apis[0].errors[0];

// ---------------------------------------------------------------------------
// Runtime helpers and shared arguments.
// ---------------------------------------------------------------------------

type JsonKind = 'string' | 'number' | 'boolean' | 'object' | 'array' | 'null';

/**
 * Assert a payload's exact key set and each value's JSON kind. Key sets are
 * compared both directions (extra and missing keys both fail), which is the
 * property serde and the TS interfaces both guarantee.
 */
function shape(label: string, value: unknown, spec: [string, JsonKind][]): void {
  expect(value, `${label} is an object`).toBeTypeOf('object');
  expect(value, `${label} is not an array`).not.toBeInstanceOf(Array);
  expect(value, `${label} is not null`).not.toBeNull();

  const record = value as Record<string, unknown>;
  expect(Object.keys(record).sort(), `${label} keys`).toEqual(
    spec.map(([key]) => key).sort(),
  );
  for (const [key, kind] of spec) {
    const item = record[key];
    const where = `${label}.${key}`;
    if (kind === 'array') {
      expect(Array.isArray(item), where).toBe(true);
    } else if (kind === 'null') {
      expect(item, where).toBeNull();
    } else if (kind === 'object') {
      expect(item, where).toBeTypeOf('object');
      expect(item, where).not.toBeNull();
      expect(Array.isArray(item), where).toBe(false);
    } else {
      expect(typeof item, where).toBe(kind);
    }
  }
}

/**
 * WHY a hand-built empty topology instead of `golden.payloads.topology`:
 * the fixture's node kinds are widened to `string` (see the value-level
 * pins above), so it is not assignable to `Topology` without a cast, and
 * the argument-key contract this file pins is identical for any instance.
 * Passing the same constant everywhere also keeps every wrapper call's
 * expected payload keys stable across the table below.
 */
const TOPOLOGY: Topology = { nodes: [], edges: [] };

/**
 * The fixture document, cast for the same widening reason. Key-set equality
 * -- what the pins above and this file's runtime checks assert -- is
 * unaffected by the cast; value fidelity of this instance is enforced by
 * Rust's deserialize roundtrip (contract_tests.rs test 7).
 */
const DOC = golden.payloads.systemDesignDoc as unknown as SystemDesignDoc;

const commandArgs = golden.commandArgs as unknown as Record<
  string,
  { all: string[]; required: string[] }
>;

/** Every payload name in `payloads`, alphabetically irrelevant (sorted on compare). */
const PAYLOAD_NAMES = [
  'appErrorPayload',
  'backupResultErr',
  'backupResultOk',
  'challengeStart',
  'challengeSummary',
  'designParseResultErr',
  'designParseResultOk',
  'glossaryEntry',
  'preset',
  'presetSummary',
  'saveResultErr',
  'saveResultOk',
  'savedDesign',
  'savedSummary',
  'simSnapshot',
  'systemDesignDoc',
  'tickError',
  'topology',
  'validationIssue',
  'vendor',
] as const;

/**
 * The wrapper -> command table. One entry per `invoke()` site in
 * `src/lib/api/*.ts` (31 of the 35 registered commands; the other four are
 * `uninvokedCommands`). The completeness test below fails if a new wrapper
 * lands without an entry here, so this table cannot silently fall behind.
 */
interface WrapperCall {
  command: string;
  call: () => Promise<unknown>;
}

const WRAPPER_CALLS: WrapperCall[] = [
  // commands/sim.rs (9)
  { command: 'sim_new', call: () => simNew(TOPOLOGY, golden.argSamples.seed) },
  { command: 'sim_set_topology', call: () => simSetTopology(TOPOLOGY) },
  {
    command: 'sim_update_node_config',
    call: () => simUpdateNodeConfig(golden.argSamples.nodeId, golden.argSamples.patch),
  },
  {
    command: 'sim_inject_failure',
    call: () =>
      simInjectFailure(
        golden.argSamples.nodeId,
        golden.argSamples.kind as FailureKind,
        golden.argSamples.opts,
      ),
  },
  { command: 'sim_clear_failure', call: () => simClearFailure(golden.argSamples.nodeId) },
  { command: 'sim_reset', call: () => simReset() },
  { command: 'sim_step', call: () => simStep(golden.argSamples.deltaMs) },
  { command: 'sim_set_running', call: () => simSetRunning(golden.argSamples.running) },
  { command: 'sim_get_snapshot', call: () => simGetSnapshot() },
  // commands/designs.rs (9)
  { command: 'designs_list', call: () => designsList() },
  { command: 'designs_save', call: () => designsSave(golden.argSamples.name, TOPOLOGY) },
  { command: 'designs_get', call: () => designsGet(golden.argSamples.id) },
  { command: 'designs_delete', call: () => designsDelete(golden.argSamples.id) },
  {
    command: 'designs_rename',
    call: () => designsRename(golden.argSamples.id, golden.argSamples.name),
  },
  {
    command: 'design_file_build',
    call: () => designFileBuild(TOPOLOGY, golden.argSamples.name),
  },
  { command: 'design_file_parse', call: () => designFileParse(golden.argSamples.text) },
  {
    command: 'design_file_write',
    call: () =>
      designFileWrite(golden.argSamples.path, TOPOLOGY, golden.argSamples.name),
  },
  { command: 'design_file_read', call: () => designFileRead(golden.argSamples.path) },
  // commands/backup.rs (2)
  { command: 'backup_write', call: () => backupWrite(golden.argSamples.path) },
  {
    command: 'backup_restore_from_path',
    call: () => backupRestoreFromPath(golden.argSamples.path),
  },
  // commands/sysdesign.rs (5)
  { command: 'sysdesign_derive_high_level', call: () => sysdesignDeriveHighLevel(TOPOLOGY) },
  { command: 'sysdesign_save', call: () => sysdesignSave(DOC) },
  { command: 'sysdesign_load', call: () => sysdesignLoad(golden.argSamples.designId) },
  { command: 'sysdesign_validate', call: () => sysdesignValidate(DOC) },
  {
    command: 'sysdesign_export',
    call: () =>
      sysdesignExport(DOC, golden.argSamples.format as ExportFormat, golden.argSamples.path),
  },
  // commands/vendors.rs (1)
  {
    command: 'vendors_get',
    call: () => vendorsGet(golden.argSamples.vendorId as VendorId),
  },
  // commands/glossary.rs (1)
  { command: 'glossary_list', call: () => glossaryList() },
  // commands/presets.rs (2)
  { command: 'presets_list', call: () => presetsList() },
  { command: 'preset_load', call: () => presetLoad(golden.argSamples.id) },
  // commands/challenges.rs (2)
  { command: 'challenges_list', call: () => challengesList() },
  { command: 'challenge_start', call: () => challengeStart(golden.argSamples.id) },
];

/**
 * Runtime key-shape rows for every payload. The nested leaf structs are
 * covered exhaustively by the type pins above and by Rust's recursive
 * key-shape roundtrip (contract_tests.rs test 7), so these rows assert the
 * levels a caller pattern-matches on -- plus the note/section annotation
 * elements, which Rust cannot pin because `Annotation` is a `Value` there.
 */
const SHAPES: { label: string; value: unknown; spec: [string, JsonKind][] }[] = [
  {
    label: 'payloads.simSnapshot',
    value: golden.payloads.simSnapshot,
    spec: [
      ['system', 'object'],
      ['nodes', 'object'],
      ['history', 'array'],
      ['edgeFlow', 'object'],
      ['edgeState', 'object'],
      ['failuresByReason', 'object'],
      ['activeFailures', 'array'],
      ['trace', 'object'],
    ],
  },
  {
    label: 'payloads.simSnapshot.system',
    value: golden.payloads.simSnapshot.system,
    spec: [
      ['timeMs', 'number'],
      ['offeredRps', 'number'],
      ['goodputRps', 'number'],
      ['errorRate', 'number'],
      ['p50', 'number'],
      ['p95', 'number'],
      ['p99', 'number'],
      ['totalRequests', 'number'],
      ['totalFailed', 'number'],
    ],
  },
  {
    label: 'payloads.simSnapshot.history[0]',
    value: golden.payloads.simSnapshot.history[0],
    spec: [
      ['t', 'number'],
      ['p50', 'number'],
      ['p95', 'number'],
      ['p99', 'number'],
      ['goodput', 'number'],
      ['offered', 'number'],
      ['errorRate', 'number'],
    ],
  },
  {
    label: 'payloads.simSnapshot.trace',
    value: golden.payloads.simSnapshot.trace,
    spec: [
      ['startMs', 'number'],
      ['totalMs', 'number'],
      ['ok', 'boolean'],
      ['reason', 'string'],
      ['hops', 'array'],
    ],
  },
  {
    label: 'payloads.simSnapshot.trace.hops[0]',
    value: golden.payloads.simSnapshot.trace.hops[0],
    spec: [
      ['nodeId', 'string'],
      ['depth', 'number'],
      ['queuedMs', 'number'],
      ['serviceMs', 'number'],
    ],
  },
  {
    label: 'payloads.simSnapshot.activeFailures[0]',
    value: golden.payloads.simSnapshot.activeFailures[0],
    spec: [
      ['nodeId', 'string'],
      ['kind', 'string'],
      ['sinceMs', 'number'],
      ['factor', 'number'],
      ['rate', 'number'],
      ['edgeIds', 'array'],
    ],
  },
  {
    label: 'payloads.topology',
    value: golden.payloads.topology,
    spec: [
      ['nodes', 'array'],
      ['edges', 'array'],
      ['annotations', 'array'],
    ],
  },
  {
    label: 'payloads.topology.nodes[0]',
    value: golden.payloads.topology.nodes[0],
    spec: [
      ['id', 'string'],
      ['kind', 'string'],
      ['label', 'string'],
      ['x', 'number'],
      ['y', 'number'],
      ['config', 'object'],
    ],
  },
  {
    label: 'payloads.topology.edges[0]',
    value: golden.payloads.topology.edges[0],
    spec: [
      ['id', 'string'],
      ['from', 'string'],
      ['to', 'string'],
      ['weight', 'number'],
      ['control', 'boolean'],
      ['latencyMs', 'number'],
      ['bandwidthRps', 'number'],
      ['lossRate', 'number'],
    ],
  },
  // The annotation element key sets: pinned here AND at compile time
  // because Rust declares `Annotation` as `serde_json::Value`
  // (contract_tests.rs header, KNOWN GAPS).
  {
    label: 'payloads.topology.annotations[note]',
    value: golden.payloads.topology.annotations[0],
    spec: [
      ['id', 'string'],
      ['kind', 'string'],
      ['text', 'string'],
      ['x', 'number'],
      ['y', 'number'],
      ['width', 'number'],
      ['size', 'string'],
      ['scale', 'number'],
      ['font', 'string'],
      ['tone', 'number'],
      ['bold', 'boolean'],
      ['italic', 'boolean'],
      ['underline', 'boolean'],
    ],
  },
  {
    label: 'payloads.topology.annotations[section]',
    value: golden.payloads.topology.annotations[1],
    spec: [
      ['id', 'string'],
      ['kind', 'string'],
      ['label', 'string'],
      ['x', 'number'],
      ['y', 'number'],
      ['width', 'number'],
      ['height', 'number'],
      ['tone', 'number'],
    ],
  },
  {
    label: 'payloads.systemDesignDoc',
    value: golden.payloads.systemDesignDoc,
    spec: [
      ['id', 'string'],
      ['name', 'string'],
      ['schemaVersion', 'number'],
      ['createdAt', 'string'],
      ['updatedAt', 'string'],
      ['topology', 'object'],
      ['architecture', 'object'],
      ['lowLevel', 'object'],
    ],
  },
  {
    label: 'payloads.systemDesignDoc.architecture',
    value: golden.payloads.systemDesignDoc.architecture,
    spec: [
      ['summary', 'string'],
      ['components', 'array'],
      ['dataFlows', 'array'],
      ['externalDependencies', 'array'],
      ['qualityAttributes', 'object'],
    ],
  },
  {
    label: 'payloads.systemDesignDoc.lowLevel',
    value: golden.payloads.systemDesignDoc.lowLevel,
    spec: [
      ['apis', 'array'],
      ['entities', 'array'],
      ['sequences', 'array'],
      ['stateMachines', 'array'],
      ['deployment', 'object'],
    ],
  },
  { label: 'payloads.savedSummary[0]', value: golden.payloads.savedSummary[0], spec: [['id', 'string'], ['name', 'string'], ['savedAt', 'number'], ['nodeCount', 'number']] },
  { label: 'payloads.savedDesign', value: golden.payloads.savedDesign, spec: [['id', 'string'], ['name', 'string'], ['savedAt', 'number'], ['topology', 'object']] },
  { label: 'payloads.saveResultOk', value: golden.payloads.saveResultOk, spec: [['ok', 'boolean'], ['id', 'string'], ['evicted', 'null']] },
  { label: 'payloads.saveResultErr', value: golden.payloads.saveResultErr, spec: [['ok', 'boolean'], ['error', 'string']] },
  { label: 'payloads.designParseResultOk', value: golden.payloads.designParseResultOk, spec: [['ok', 'boolean'], ['topology', 'object'], ['name', 'string']] },
  { label: 'payloads.designParseResultErr', value: golden.payloads.designParseResultErr, spec: [['ok', 'boolean'], ['error', 'string']] },
  { label: 'payloads.backupResultOk', value: golden.payloads.backupResultOk, spec: [['ok', 'boolean'], ['restored', 'array']] },
  { label: 'payloads.backupResultErr', value: golden.payloads.backupResultErr, spec: [['ok', 'boolean'], ['error', 'string']] },
  { label: 'payloads.validationIssue[0]', value: golden.payloads.validationIssue[0], spec: [['field', 'string'], ['message', 'string']] },
  { label: 'payloads.vendor', value: golden.payloads.vendor, spec: [['id', 'string'], ['label', 'string'], ['region', 'string'], ['kinds', 'object']] },
  {
    label: 'payloads.glossaryEntry[0]',
    value: golden.payloads.glossaryEntry[0],
    spec: [
      ['id', 'string'],
      ['term', 'string'],
      ['short', 'string'],
      ['why', 'string'],
      ['category', 'string'],
      ['see', 'array'],
      ['aliases', 'array'],
    ],
  },
  { label: 'payloads.presetSummary[0]', value: golden.payloads.presetSummary[0], spec: [['id', 'string'], ['name', 'string'], ['tagline', 'string'], ['description', 'string']] },
  { label: 'payloads.preset', value: golden.payloads.preset, spec: [['id', 'string'], ['name', 'string'], ['tagline', 'string'], ['description', 'string'], ['topology', 'object']] },
  { label: 'payloads.challengeSummary[0]', value: golden.payloads.challengeSummary[0], spec: [['id', 'string'], ['name', 'string'], ['brief', 'string']] },
  { label: 'payloads.challengeStart', value: golden.payloads.challengeStart, spec: [['challenge', 'object'], ['topology', 'object']] },
  {
    label: 'payloads.challengeStart.challenge',
    value: golden.payloads.challengeStart.challenge,
    spec: [
      ['id', 'string'],
      ['name', 'string'],
      ['brief', 'string'],
      ['presetId', 'string'],
      ['loadRps', 'number'],
      ['goals', 'array'],
      ['hints', 'array'],
      ['lesson', 'string'],
    ],
  },
  { label: 'payloads.challengeStart.challenge.goals[0]', value: golden.payloads.challengeStart.challenge.goals[0], spec: [['metric', 'string'], ['max', 'number']] },
  { label: 'payloads.appErrorPayload', value: golden.payloads.appErrorPayload, spec: [['kind', 'string'], ['message', 'string']] },
  { label: 'payloads.tickError', value: golden.payloads.tickError, spec: [['message', 'string']] },
];

describe('IPC contract against contract/ipc-golden.json', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    listenMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
    listenMock.mockResolvedValue(() => {});
  });

  describe('the fixture itself is internally consistent', () => {
    it('the bundled JSON import and the file on disk are the same bytes', () => {
      // If these ever differ, one suite would be testing a fixture the
      // other never saw (the Rust side embeds the file with include_str!).
      expect(golden).toEqual(goldenJson);
    });

    it('commandArgs lists exactly the registered command names', () => {
      expect(Object.keys(golden.commandArgs).sort()).toEqual([...golden.commands].sort());
    });

    it('payload inventory matches the payload names this suite covers', () => {
      expect(Object.keys(golden.payloads).sort()).toEqual([...PAYLOAD_NAMES].sort());
    });

    it('uninvokedCommands are the four settings/layout commands with no TS caller', () => {
      expect([...golden.uninvokedCommands].sort()).toEqual([
        'layout_load',
        'layout_save',
        'settings_load',
        'settings_save',
      ]);
    });

    it('value-level pins reference the fixture instances themselves', () => {
      expect(valuePinSystem).toBe(golden.payloads.simSnapshot.system);
      expect(valuePinHistory).toBe(golden.payloads.simSnapshot.history[0]);
      expect(valuePinIssue).toBe(golden.payloads.validationIssue[0]);
      expect(valuePinSavedSummary).toBe(golden.payloads.savedSummary[0]);
      expect(valuePinQuality).toBe(
        golden.payloads.systemDesignDoc.architecture.qualityAttributes,
      );
      expect(valuePinApiError).toBe(
        golden.payloads.systemDesignDoc.lowLevel.apis[0].errors[0],
      );
    });
  });

  describe('payload wire shapes', () => {
    it.each(SHAPES)('$label key set and value kinds', ({ label, value, spec }) => {
      shape(label, value, spec);
    });

    it('annotation discriminators are the two kinds the canvas draws', () => {
      expect(golden.payloads.topology.annotations[0].kind).toBe('note');
      expect(golden.payloads.topology.annotations[1].kind).toBe('section');
    });
  });

  describe('wrapper invocations', () => {
    it.each(WRAPPER_CALLS)('invokes $command with the fixture argument keys', async ({
      command,
      call,
    }) => {
      await call();

      expect(invokeMock, `${command} must be invoked exactly once`).toHaveBeenCalledTimes(1);
      const [invoked, args] = invokeMock.mock.calls[0];
      expect(invoked, 'command name').toBe(command);

      const spec = commandArgs[command];
      const keys = args === undefined ? [] : Object.keys(args).sort();
      expect(keys, `${command} argument keys`).toEqual([...spec.all].sort());
      // Cast: InvokeArgs = Record | number[] | ArrayBuffer | Uint8Array;
      // only the Record member is string-indexable, and every wrapper
      // passes a plain object.
      const payload = args as Record<string, unknown> | undefined;
      for (const key of spec.required) {
        // A key present but `undefined` is dropped by structured clone, so
        // Rust would see a payload missing a required field at runtime.
        expect(payload?.[key], `${command}.${key} must be present and not undefined`).toBeDefined();
      }
    });

    it('covers exactly the commands that have a TS wrapper', () => {
      const covered = WRAPPER_CALLS.map((entry) => entry.command);
      expect(new Set(covered).size, 'every command appears once').toBe(covered.length);
      const expected = golden.commands.filter(
        (name) => !golden.uninvokedCommands.includes(name),
      );
      expect([...covered].sort()).toEqual([...expected].sort());
    });
  });

  describe('event channels', () => {
    it('subscribes to exactly the channels the fixture declares', async () => {
      await onSnapshot(() => {});
      await onTickError(() => {});

      expect(listenMock.mock.calls.map((call) => call[0]).sort()).toEqual(
        Object.keys(golden.events).sort(),
      );
      expect(listenMock).toHaveBeenCalledTimes(2);
    });

    it('delivers the fixture payloads through the subscription callbacks', async () => {
      const snapshots = vi.fn();
      const tickErrors = vi.fn();
      await onSnapshot(snapshots);
      await onTickError(tickErrors);

      const snapshotHandler = listenMock.mock.calls[0][1] as unknown as (event: {
        payload: unknown;
      }) => void;
      const tickErrorHandler = listenMock.mock.calls[1][1] as unknown as (event: {
        payload: unknown;
      }) => void;

      snapshotHandler({ payload: golden.payloads.simSnapshot });
      tickErrorHandler({ payload: golden.payloads.tickError });

      expect(snapshots).toHaveBeenCalledWith(golden.payloads.simSnapshot);
      expect(tickErrors).toHaveBeenCalledWith(golden.payloads.tickError.message);
    });
  });
});
