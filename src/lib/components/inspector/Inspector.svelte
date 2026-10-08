<script lang="ts">
	/* ==========================================================================
	   Inspector.

	   Data-driven Svelte port of src/components/Inspector.tsx (3,260 lines).
	   The web app hand-writes one JSX field list per NodeKind inside a giant
	   switch; this component instead renders whatever `field-schema.ts` says
	   applies to the selected node's kind with ONE generic loop. See that
	   module's header for exactly what was transcribed from the web app
	   (FIELDS_BY_KIND / FIELD_SPECS / KIND_FIELD_OVERRIDES, which are the
	   authoritative ranges an instructor actually tuned) versus written fresh
	   for this port (the client's traffic-pattern section, which the web app
	   never wired into any control at all).

	   STRUCTURE, per node selected:
	     1. Empty state when nothing is selected (topologyStore.selectedNodeId
	        is null, or points at a node that no longer exists).
	     2. Header: node label, kind name, one-line blurb.
	     3. "Vitals": a small pill row of the node's live headline numbers,
	        only once simulationStore.snapshot has produced stats for this
	        node -- absent (not zeroed) while the engine has not started yet,
	        per MIGRATION_PLAN's loading-state note.
	     4. "Suggested fix": one grounded suggestion, shown only once this
	        node's headroom (spare capacity -- field-schema's `headroomFor`)
	        drops below 1.0x, i.e. it cannot keep up with what is arriving.
	        Port of upstream `eb163673`/PR #71; text lives in
	        `$lib/content/suggestions.ts`, gating in `headroomFor`/
	        `HAS_THROUGHPUT_CEILING` (field-schema.ts).
	     5. For `client` only: a dedicated traffic section (rps + the traffic
	        pattern picker + its period), standing in for the web app's
	        separately-imported `TrafficControl`.
	     6. Every other applicable field, grouped exactly as the web app
	        groups them ("What it does" / "How much it can handle" / "When
	        things go wrong"), each field's live NodeStats reading shown
	        directly under its control when the engine has one to show.

	   COMMIT TIMING. Sliders update a local `draft` value on every `input`
	   (so the readout and the fill-percent track the drag instantly) but only
	   call `updateNodeConfig` -- which pushes to the Rust engine over IPC --
	   on `change` (pointer-up, or Enter/blur for a typed value). Plain number
	   inputs and the traffic-pattern select only ever fire `change`. This
	   keeps a drag from flooding IPC with one call per pixel of travel without
	   needing a custom debounce; MIGRATION_PLAN explicitly says exact
	   live-drag timing does not need to match the web app's.
	   ========================================================================== */

	import { topologyStore, updateNodeConfig, clearSelection } from '$lib/state/topology.svelte';
	import { simulationStore } from '$lib/state/simulation.svelte';
	import { DEFAULT_TRAFFIC_PERIOD_S } from '$lib/domain';
	import type { NodeConfig, NodeStats, SimNode, TrafficPattern } from '$lib/domain';
	import {
		FIELDS_BY_KIND,
		FIELD_GROUPS,
		GATE_KINDS,
		KIND_BLURB,
		KIND_NAME,
		TRAFFIC_FIELDS,
		specFor,
		fillPct,
		formatFieldValue,
		headroomFor,
		liveReadout,
		pct,
		rate,
		type InspectorField,
		type RangeFieldSpec,
	} from './field-schema';
	import { suggestionFor } from '$lib/content/suggestions';
	import { sessionHistory, currentSnapshot } from '$lib/state/history.svelte';

	/* ---------------------------------------------------------------- *
	 * Selection -> node -> stats.
	 * ---------------------------------------------------------------- */

	const node = $derived<SimNode | null>(
		topologyStore.topology.nodes.find((n) => n.id === topologyStore.selectedNodeId) ?? null,
	);

	const stats = $derived<NodeStats | null>(
		node && simulationStore.snapshot ? (simulationStore.snapshot.nodes[node.id] ?? null) : null,
	);

	const fieldsForKind = $derived<InspectorField[]>(node ? FIELDS_BY_KIND[node.kind] : []);

	/** Spare capacity as a multiple of arrivals, or null when the kind has no
	 * throughput ceiling or nothing has arrived yet -- see `headroomFor`. */
	const headroom = $derived<number | null>(node ? headroomFor(node, stats) : null);

	/** One grounded suggestion, only once this node cannot keep up (headroom
	 * < 1x) -- the same threshold the web app's "Spare capacity" reading is
	 * toned by. Port of upstream `eb163673`/PR #71. */
	const suggestion = $derived<string | null>(
		node && headroom !== null && headroom < 1 ? suggestionFor(node.kind) : null,
	);

	/** Every applicable field except the three the dedicated traffic section owns. */
	const bodyFields = $derived<InspectorField[]>(
		node && node.kind === 'client'
			? fieldsForKind.filter((f) => !TRAFFIC_FIELDS.includes(f))
			: fieldsForKind,
	);

	const grouped = $derived(
		FIELD_GROUPS.map((g) => ({
			title: g.title,
			fields: bodyFields.filter((f) => g.fields.has(f)),
		})).filter((g) => g.fields.length > 0),
	);

	/* ---------------------------------------------------------------- *
	 * Draft values: instant local feedback while a slider drags, without
	 * calling updateNodeConfig until the drag (or a typed value) commits.
	 * Keyed by field name; cleared whenever the selected node changes so a
	 * stale drag from a previous node can never leak onto a new one.
	 * ---------------------------------------------------------------- */

	let draft = $state<Partial<Record<InspectorField, number>>>({});
	let draftForId = $state<string | null>(null);

	$effect(() => {
		const id = node?.id ?? null;
		if (id !== draftForId) {
			draft = {};
			draftForId = id;
		}
	});

	/** Older/partial configs can omit an optional field; fall back to this
	 * kind's spec minimum rather than to `undefined`, so the control always
	 * has a real value to show instead of silently defaulting to NaN. */
	function rawNumber(n: SimNode, field: InspectorField): number {
		const v = (n.config as unknown as Record<string, unknown>)[field];
		if (typeof v === 'number' && Number.isFinite(v)) return v;
		const spec = specFor(n.kind, field);
		return spec.control === 'enum' ? 0 : spec.min;
	}

	function valueOf(n: SimNode, field: InspectorField): number {
		return draft[field] ?? rawNumber(n, field);
	}

	function commit(field: InspectorField, value: number): void {
		if (!node) return;
		const next = { ...draft };
		delete next[field];
		draft = next;
		// Baseline BEFORE the write; touch (not commit) so successive knob
		// changes inside the settle window land as ONE entry -- App.tsx's
		// handleConfigChange touches 'setting change' the same way (:1751).
		sessionHistory.touch('setting change', currentSnapshot());
		updateNodeConfig(node.id, { [field]: value } as Partial<NodeConfig>);
	}

	function onSliderInput(field: InspectorField, e: Event): void {
		const v = Number((e.currentTarget as HTMLInputElement).value);
		draft = { ...draft, [field]: v };
	}

	function onSliderCommit(field: InspectorField, e: Event): void {
		commit(field, Number((e.currentTarget as HTMLInputElement).value));
	}

	function onNumberCommit(field: InspectorField, e: Event): void {
		const input = e.currentTarget as HTMLInputElement;
		if (!node) return;
		const spec = specFor(node.kind, field) as RangeFieldSpec;
		const raw = input.value.trim();
		const typed = Number(raw);

		/* A typed commit can be an emptied box, non-numeric text, or a number
		 * outside the field's own min/max -- `type="number"` reports all three
		 * happily, and `Number('')` is 0 while `Number('abc')` is NaN. Any of
		 * them used to travel to `updateNodeConfig`, and the Rust side merges
		 * the patch with `serde_json::from_value::<NodeConfig>` (engine.rs),
		 * which rejects the `null` that JSON gives a NaN. By then the
		 * optimistic store already showed the value, so the Inspector
		 * displayed a number the engine had never taken, and it stayed wrong
		 * until the next node selection. (This is the desktop form of Bug 4;
		 * the web app had no guard either, because its store write is the
		 * same optimistic path -- the difference is nothing ever tells it no.)
		 *
		 * Empty/NaN: no write at all, and the box is restored to the node's
		 * live value so what you see is what the engine has. Out of range:
		 * clamp to the spec's min/max -- the input's own `min`/`max`
		 * attributes already promise that interval, so the typed number is
		 * brought back into it rather than sent off to be rejected. */
		if (raw === '' || Number.isNaN(typed)) {
			input.value = String(rawNumber(node, field));
			const next = { ...draft };
			delete next[field];
			draft = next;
			return;
		}
		const value = Math.min(spec.max, Math.max(spec.min, typed));
		if (value !== typed) input.value = String(value);
		commit(field, value);
	}

	function onTrafficPatternChange(e: Event): void {
		if (!node) return;
		const value = (e.currentTarget as HTMLSelectElement).value as TrafficPattern;
		// Same 'setting change' stream as every other knob here (App.tsx
		// :1776/:1829 use one label for all config paths, so a pattern flip
		// coalesces with a slider drag into one undo step).
		sessionHistory.touch('setting change', currentSnapshot());
		updateNodeConfig(node.id, { traffic: value });
	}

	/** Current string value of an `'enum'`-control field (so far only the
	 * bulkhead's `bulkheadMode`), falling back to the spec's first option --
	 * the same "older config omits an optional field" reasoning `rawNumber`
	 * documents above, just for a string-valued field instead of a numeric
	 * one. Generic over `field` so the grouped loop below needs no
	 * per-field special case, the same way `onTrafficPatternChange`'s
	 * hand-written client-only section is special-cased instead. Port of
	 * the web app's `ChoiceRow` (upstream `351327c4`, PR #77).
	 */
	function enumValueOf(n: SimNode, field: InspectorField): string {
		const v = (n.config as unknown as Record<string, unknown>)[field];
		if (typeof v === 'string') return v;
		const spec = specFor(n.kind, field);
		return spec.control === 'enum' ? (spec.options[0]?.value ?? '') : '';
	}

	function onEnumCommit(field: InspectorField, e: Event): void {
		if (!node) return;
		const value = (e.currentTarget as HTMLSelectElement).value;
		sessionHistory.touch('setting change', currentSnapshot());
		updateNodeConfig(node.id, { [field]: value } as Partial<NodeConfig>);
	}

	/* ---------------------------------------------------------------- *
	 * Header vitals: a handful of pills, not a full metrics readout (that
	 * is the metrics-strip/trace panels' job). Gate kinds (rate limiter,
	 * breaker, load shedder, bulkhead) hold no work of their own, so their
	 * throughput/in-flight numbers are structurally zero and are skipped
	 * rather than shown as a real reading -- see GATE_KINDS in field-schema.
	 * ---------------------------------------------------------------- */

	function toneFor(v: number | undefined, warn: number, danger: number): string {
		if (v === undefined || !Number.isFinite(v)) return '';
		if (v >= danger) return 'is-danger';
		if (v >= warn) return 'is-warn';
		return 'is-ok';
	}
</script>

<aside class="ins-panel panel" aria-label="Inspector">
	<div class="ins-scroll scroll">
		{#if !node}
			<div class="empty">
				<p>Select a component to configure it.</p>
			</div>
		{:else}
			{@const n = node}
			{@const kindName = KIND_NAME[n.kind]}
			<header class="ins-head">
				<h2 class="ins-title truncate">{n.label}</h2>
				{#if n.label.trim().toLowerCase() !== kindName.toLowerCase()}
					<span class="label">{kindName}</span>
				{/if}
				<!-- Escape and a background click also close the inspector, but
				     both are invisible: students reported being trapped here. -->
				<button
					type="button"
					class="btn btn-ghost btn-sm btn-icon ins-close"
					aria-label="Close inspector"
					onclick={() => clearSelection()}
				>
					<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M18 6 6 18M6 6l12 12" /></svg>
				</button>
			</header>
			<p class="prose ins-blurb">{KIND_BLURB[n.kind]}</p>

			{#if !simulationStore.snapshot}
				<p class="prose ins-loading">Configuration only -- the simulation engine has not started yet.</p>
			{/if}

			{#if stats}
				<div class="ins-vitals">
					{#if stats.instances !== undefined && pct(stats.utilization)}
						<span class="pill {toneFor(stats.utilization, 0.7, 0.9)}">
							Busy {pct(stats.utilization)}
						</span>
					{/if}
					{#if !GATE_KINDS.has(n.kind) && rate(stats.throughput)}
						<span class="pill">Throughput {rate(stats.throughput)}</span>
					{/if}
					{#if pct(stats.errorRate)}
						<span class="pill {toneFor(stats.errorRate, 0.01, 0.05)}">
							Errors {pct(stats.errorRate)}
						</span>
					{/if}
					{#if stats.p99 > 0}
						<span class="pill">p99 {Math.round(stats.p99)}ms</span>
					{/if}
				</div>
			{/if}

			{#if suggestion}
				<section class="ins-section">
					<h3 class="label">Suggested fix</h3>
					<p class="prose ins-suggestion">{suggestion}</p>
				</section>
			{/if}

			{#if n.kind === 'client'}
				{@const rpsSpec = specFor(n.kind, 'rps') as RangeFieldSpec}
				{@const rpsVal = valueOf(n, 'rps')}
				{@const patternSpec = specFor(n.kind, 'traffic')}
				{@const pattern = n.config.traffic ?? 'steady'}
				<section class="ins-section">
					<h3 class="label">Traffic</h3>
					<div class="ins-fields">
						<div class="field-stack ins-row">
							<div class="mx-head">
								<label class="row-k" for="ins-rps">{rpsSpec.label}</label>
								<span class="row-v">{formatFieldValue(rpsSpec, rpsVal)}</span>
							</div>
							<input
								id="ins-rps"
								class="slider"
								type="range"
								min={rpsSpec.min}
								max={rpsSpec.max}
								step={rpsSpec.step}
								value={rpsVal}
								style={`--fill-pct: ${fillPct(rpsSpec, rpsVal)}`}
								aria-describedby="ins-rps-u"
								oninput={(e) => onSliderInput('rps', e)}
								onchange={(e) => onSliderCommit('rps', e)}
							/>
							<span id="ins-rps-u" class="sr-only">{rpsSpec.unit}</span>
						</div>
						{#if rpsSpec.hint}<p class="prose ins-hint">{rpsSpec.hint}</p>{/if}

						<div class="field">
							<label class="row-k" for="ins-traffic-pattern">{patternSpec.label}</label>
							<select
								id="ins-traffic-pattern"
								class="ins-select-wide"
								value={pattern}
								onchange={onTrafficPatternChange}
							>
								{#if patternSpec.control === 'enum'}
									{#each patternSpec.options as opt (opt.value)}
										<option value={opt.value}>{opt.label}</option>
									{/each}
								{/if}
							</select>
						</div>
						{#if patternSpec.hint}<p class="prose ins-hint">{patternSpec.hint}</p>{/if}

						{#if pattern !== 'steady'}
							{@const periodSpec = specFor(n.kind, 'trafficPeriodS') as RangeFieldSpec}
							{@const periodVal = n.config.trafficPeriodS ?? DEFAULT_TRAFFIC_PERIOD_S}
							<div class="field">
								<label class="row-k" for="ins-traffic-period">{periodSpec.label}</label>
								<input
									id="ins-traffic-period"
									type="number"
									min={periodSpec.min}
									max={periodSpec.max}
									step={periodSpec.step}
									value={periodVal}
									onchange={(e) => onNumberCommit('trafficPeriodS', e)}
								/>
							</div>
							{#if periodSpec.hint}<p class="prose ins-hint">{periodSpec.hint}</p>{/if}
						{/if}
					</div>
				</section>
			{/if}

			{#each grouped as group (group.title)}
				<section class="ins-section">
					<h3 class="label">{group.title}</h3>
					<div class="ins-fields">
						{#each group.fields as field (field)}
							{@const spec = specFor(n.kind, field)}
							{@const value = valueOf(n, field)}
							{@const live = liveReadout(field, stats)}
							{#if spec.control === 'number'}
								<div class="field">
									<label class="row-k" for={`ins-${field}`}>{spec.label}</label>
									<input
										id={`ins-${field}`}
										type="number"
										min={spec.min}
										max={spec.max}
										step={spec.step}
										value={value}
										onchange={(e) => onNumberCommit(field, e)}
									/>
								</div>
							{:else if spec.control === 'enum'}
								<div class="field">
									<label class="row-k" for={`ins-${field}`}>{spec.label}</label>
									<select
										id={`ins-${field}`}
										class="ins-select-wide"
										value={enumValueOf(n, field)}
										onchange={(e) => onEnumCommit(field, e)}
									>
										{#each spec.options as opt (opt.value)}
											<option value={opt.value}>{opt.label}</option>
										{/each}
									</select>
								</div>
							{:else}
								<div class="field-stack ins-row">
									<div class="mx-head">
										<label class="row-k" for={`ins-${field}`}>{spec.label}</label>
										<span class="row-v">{formatFieldValue(spec, value)}</span>
									</div>
									<input
										id={`ins-${field}`}
										class="slider"
										type="range"
										min={spec.min}
										max={spec.max}
										step={spec.step}
										value={value}
										style={`--fill-pct: ${fillPct(spec, value)}`}
										aria-describedby={`ins-${field}-u`}
										oninput={(e) => onSliderInput(field, e)}
										onchange={(e) => onSliderCommit(field, e)}
									/>
									<span id={`ins-${field}-u`} class="sr-only">{spec.unit}</span>
								</div>
							{/if}
							{#if spec.hint}<p class="prose ins-hint">{spec.hint}</p>{/if}
							{#if live}<p class="ins-live">{live}</p>{/if}
						{/each}
					</div>
				</section>
			{/each}
		{/if}
	</div>
</aside>

<style>
	/* ==========================================================================
	   Inspector panel. Every control reuses app.css's .field/.field-stack/
	   .row-k/.row-v/.slider/.pill/inputs primitives -- see that file's own
	   header comment: they exist specifically so a per-field config editor
	   like this one needs no control styling of its own. What is defined
	   here is layout/composition only: the panel shell, section spacing, and
	   the small bits app.css has no primitive for (the vitals pill row, the
	   live-readout line under a control).
	   ========================================================================== */

	.ins-panel {
		display: flex;
		flex-direction: column;
		min-height: 0;
		padding: var(--sp-5);
	}

	.ins-scroll {
		min-height: 0;
		display: flex;
		flex-direction: column;
	}

	.ins-head {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		gap: var(--sp-2);
		min-width: 0;
	}

	/* The head aligns baselines (title vs kind label); the close button has
	   no text baseline of its own, so centre it instead, and never let it
	   squeeze the truncating title. */
	.ins-close {
		align-self: center;
		flex: none;
	}

	.ins-title {
		margin: 0;
		font-size: var(--fs-num);
		font-weight: var(--fw-num);
		letter-spacing: var(--tr-num);
		color: var(--text);
	}

	.ins-blurb {
		margin: var(--sp-1) 0 0;
	}

	.ins-loading {
		margin: var(--sp-2) 0 0;
		color: var(--text-faint);
	}

	.ins-vitals {
		display: flex;
		flex-wrap: wrap;
		gap: var(--sp-1);
		margin-top: var(--sp-3);
	}

	.ins-section {
		display: flex;
		flex-direction: column;
		gap: var(--sp-2);
		margin-top: var(--sp-7);
	}

	.ins-fields {
		display: flex;
		flex-direction: column;
		gap: var(--sp-4);
	}

	/* A field/field-stack row followed by its own hint and/or live-readout
	   paragraph sits tighter than the ins-fields gap gives every other
	   field, so the explanation reads as attached to the control above it
	   rather than floating between two controls. */
	.ins-fields > .field + .prose,
	.ins-fields > .field-stack + .prose,
	.ins-fields > .prose + .ins-live,
	.ins-fields > .field + .ins-live,
	.ins-fields > .field-stack + .ins-live {
		margin-top: calc(var(--sp-2) * -1);
	}

	.ins-hint {
		color: var(--text-dim);
	}

	/* Teaching prose like .ins-blurb/.ins-hint, not a live reading like
	   .ins-live below -- PR #71 (eb163673) ports this as its own class so
	   the suggestion is a distinct, addressable paragraph. */
	.ins-suggestion {
		color: var(--text-dim);
	}

	.ins-live {
		margin: 0;
		font-size: var(--fs-sm);
		line-height: var(--lh-sm);
		color: var(--text-faint);
	}

	.ins-live::before {
		content: '\25CF\2002';
		font-size: 8px;
		vertical-align: middle;
		color: var(--accent);
	}

	/* The traffic-pattern select's longest option ("Diurnal (day/night)")
	   does not fit app.css's default 96px .field control width. */
	.field > select.ins-select-wide {
		flex: 0 0 180px;
		width: 180px;
	}
</style>
