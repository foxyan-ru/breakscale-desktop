<script module lang="ts">
	/**
	 * A preset's browsable summary: id, name, tagline, description -- no
	 * topology. Mirrors Rust `PresetSummary` in
	 * `src-tauri/src/sim/presets.rs` (`pub struct PresetSummary { id,
	 * name, tagline, description }`, `#[serde(rename_all = "camelCase")]`),
	 * which is what `sim::presets::all_presets()` returns -- the cheap list,
	 * without paying to load all 23 topologies. The full `Preset` (summary +
	 * `topology`) is what `sim::presets::preset_by_id(id)` returns.
	 *
	 * NOTE(integration): needs `presets_list` (-> `Vec<PresetSummary>`,
	 * backed by `sim::presets::all_presets()`) and `preset_load` (id ->
	 * `Preset`, backed by `sim::presets::preset_by_id()`) Tauri commands, plus
	 * wrappers in `desktop/src/lib/api/*.ts`. A parallel agent ported
	 * `presets.ts` to `src-tauri/data/presets/*.json` and the Rust
	 * loader functions above, but nothing exposes them over IPC yet. Until
	 * that exists, this component works only with summaries it is handed and
	 * a load-by-id callback -- see `Props` below -- so it is fully
	 * testable/reviewable now.
	 */
	export interface PresetSummary {
		id: string;
		name: string;
		tagline: string;
		description: string;
	}
</script>

<script lang="ts">
	/* ==========================================================================
	   The examples gallery.

	   Svelte port of src/components/Examples.tsx. See that file's header for
	   the full rationale (why this left the components rail and became a
	   dialog, why a gallery card beats a rail row for 23 presets).
	   ========================================================================== */

	interface Props {
		open: boolean;
		onClose: () => void;
		/** Every preset's summary. Empty is a valid (loading/no-data) state. */
		presets: PresetSummary[];
		/** True while `presets` is still being fetched from Rust. */
		loading?: boolean;
		activePresetId: string | null;
		/**
		 * Load a preset by id onto the canvas. This component only ever
		 * works with summaries (no topology), so it hands back just the id;
		 * the caller is expected to resolve the full `Preset` (via a future
		 * `preset_load` command) and apply its topology once that exists.
		 */
		onLoad: (id: string) => void;
	}

	let { open, onClose, presets, loading = false, activePresetId, onLoad }: Props = $props();

	let cardEl: HTMLDivElement | undefined = $state(undefined);
	let searchEl: HTMLInputElement | undefined = $state(undefined);
	let query = $state('');

	let mounted = $state(false);
	let closing = $state(false);

	$effect(() => {
		if (open) {
			closing = false;
			mounted = true;
		} else if (mounted) {
			closing = true;
		}
	});

	function handleAnimationEnd(e: AnimationEvent) {
		if (closing && e.target === e.currentTarget) {
			mounted = false;
			closing = false;
		}
	}

	/* Reset on OPEN rather than on close, so the list is not blanked out from
	   under the reader while the dialog is still sliding away. */
	$effect(() => {
		if (open) query = '';
	});

	/* Focus goes to the search field: with twenty-three examples the first
	   thing a returning student does is type. Focus returns to the opener. */
	$effect(() => {
		if (!open) return;
		const opener =
			document.activeElement instanceof HTMLElement ? document.activeElement : null;
		const card = cardEl;
		searchEl?.focus();
		return () => {
			const active = document.activeElement;
			const inside = card?.contains(active as Node) ?? false;
			if (inside || active === document.body || active === null) {
				opener?.focus();
			}
		};
	});

	function onKeyDown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			e.preventDefault();
			e.stopPropagation();
			onClose();
		}
	}

	function onSearchInput(e: Event) {
		query = (e.currentTarget as HTMLInputElement).value;
	}

	const shown = $derived.by(() => {
		const q = query.trim().toLowerCase();
		if (!q) return presets;
		return presets.filter(
			(p) =>
				p.name.toLowerCase().includes(q) ||
				p.tagline.toLowerCase().includes(q) ||
				p.description.toLowerCase().includes(q),
		);
	});
</script>

{#if mounted}
	<div class={`ex-root${closing ? ' is-closing' : ''}`} inert={closing || undefined}>
		<div class="ex-scrim" onclick={onClose} aria-hidden="true"></div>
		<div
			bind:this={cardEl}
			class="ex-card"
			role="dialog"
			aria-modal="true"
			aria-labelledby="ex-title"
			onkeydown={onKeyDown}
			onanimationend={handleAnimationEnd}
		>
			<header class="ex-head">
				<div>
					<h2 id="ex-title" class="ex-title">Examples</h2>
					<p class="ex-sub">Load a system that already works, then take it apart.</p>
				</div>
				<button type="button" class="btn" onclick={onClose}>Close</button>
			</header>

			<input
				bind:this={searchEl}
				type="text"
				class="ex-search"
				placeholder="Search examples"
				value={query}
				oninput={onSearchInput}
				aria-label="Search examples"
			/>

			{#if loading}
				<p class="ex-empty">Loading examples…</p>
			{:else if presets.length === 0}
				<p class="ex-empty">No examples are available yet.</p>
			{:else if shown.length === 0}
				<p class="ex-empty">
					Nothing matches "{query}". Try a component name like cache or queue.
				</p>
			{:else}
				<ul class="ex-grid">
					{#each shown as preset (preset.id)}
						{@const active = preset.id === activePresetId}
						<li>
							<button
								type="button"
								class="ex-item"
								data-active={active || undefined}
								aria-current={active ? 'true' : undefined}
								onclick={() => {
									onLoad(preset.id);
									onClose();
								}}
							>
								<span class="ex-item-name">{preset.name}</span>
								<span class="ex-item-tagline">{preset.tagline}</span>
								<!-- The full description is what actually helps a student
								     choose, and it is the thing a rail row had no room for. -->
								<span class="ex-item-desc">{preset.description}</span>
								{#if active}<span class="ex-item-active">Loaded</span>{/if}
							</button>
						</li>
					{/each}
				</ul>
			{/if}
		</div>
	</div>
{/if}

<style>
	/* ==========================================================================
	   The examples gallery. Ported from Examples.css.

	   Same dialog shell as the shortcuts card, deliberately: a student who
	   has opened one reference already knows how this one behaves.
	   ========================================================================== */

	.ex-root {
		position: fixed;
		inset: 0;
		z-index: 500;
		display: grid;
		place-items: center;
		padding: var(--sp-5);
	}

	.ex-scrim {
		position: absolute;
		inset: 0;
		background: var(--scrim);
		cursor: default;
		animation: ex-scrim-in var(--dur-slow) var(--ease-out);
	}

	.ex-root.is-closing .ex-scrim {
		animation: ex-scrim-out var(--dur-base) var(--ease) forwards;
	}

	.ex-card {
		position: relative;
		display: flex;
		flex-direction: column;
		gap: var(--sp-4);
		width: min(980px, 100%);
		max-height: min(720px, 100%);
		min-height: 0;
		padding: var(--sp-5);
		background: var(--surface);
		border: var(--bw) solid var(--border-strong);
		border-radius: var(--r-lg);
		box-shadow: var(--shadow-lg);
		outline: none;
		animation: ex-card-in var(--dur-slow) var(--ease-out);
	}

	.ex-root.is-closing .ex-card {
		animation: ex-card-out var(--dur-base) var(--ease) forwards;
	}

	.ex-head {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: var(--sp-4);
	}

	.ex-title {
		margin: 0;
		font-size: var(--fs-lg);
		font-weight: var(--fw-med);
		line-height: var(--lh-sm);
		color: var(--text);
	}

	.ex-sub {
		margin: var(--sp-1) 0 0;
		font-size: var(--fs-sm);
		font-weight: var(--fw-body);
		line-height: var(--lh-sm);
		color: var(--text-dim);
	}

	.ex-search {
		flex: 0 0 auto;
	}

	/* auto-fill rather than auto-fit: with a search filtered down to one
	   result, auto-fit would stretch that single card the full width of the
	   dialog, which reads as a layout accident. */
	.ex-grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
		gap: var(--sp-3);
		margin: 0;
		padding: 0 var(--sp-1) var(--sp-1) 0;
		list-style: none;
		overflow-y: auto;
		min-height: 0;
		scrollbar-width: thin;
		scrollbar-color: var(--line) transparent;
		overscroll-behavior: contain;
	}

	.ex-item {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: var(--sp-1);
		width: 100%;
		height: 100%;
		padding: var(--sp-3);
		text-align: left;
		background: var(--surface-2);
		border: var(--bw) solid var(--border);
		border-radius: var(--r-btn);
		cursor: pointer;
		font: inherit;
		transition:
			background-color var(--dur-fast) var(--ease),
			border-color var(--dur-fast) var(--ease);
	}

	.ex-item:hover {
		background: var(--surface-3);
		border-color: var(--border-strong);
	}

	.ex-item:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 2px;
	}

	.ex-item[data-active] {
		border-color: var(--accent);
	}

	.ex-item-name {
		font-size: var(--fs-base);
		font-weight: var(--fw-med);
		line-height: var(--lh-sm);
		color: var(--text);
	}

	.ex-item-tagline {
		font-size: var(--fs-sm);
		font-weight: var(--fw-body);
		line-height: var(--lh-sm);
		color: var(--text-dim);
	}

	.ex-item-desc {
		margin-top: var(--sp-1);
		font-size: var(--fs-sm);
		font-weight: var(--fw-body);
		line-height: var(--lh-prose);
		color: var(--text-faint);
		text-wrap: pretty;
	}

	.ex-item-active {
		margin-top: auto;
		padding-top: var(--sp-2);
		font-size: var(--fs-label);
		font-weight: var(--fw-med);
		letter-spacing: var(--tr-label);
		text-transform: uppercase;
		color: var(--accent-ink);
	}

	.ex-empty {
		margin: 0;
		padding: var(--sp-5) 0;
		font-size: var(--fs-sm);
		line-height: var(--lh-prose);
		color: var(--text-dim);
		text-align: center;
	}

	@keyframes ex-card-in {
		from {
			opacity: 0;
			transform: translateY(6px);
		}
		to {
			opacity: 1;
			transform: translateY(0);
		}
	}

	@keyframes ex-card-out {
		from {
			opacity: 1;
			transform: translateY(0);
		}
		to {
			opacity: 0;
			transform: translateY(4px);
		}
	}

	@keyframes ex-scrim-in {
		from {
			opacity: 0;
		}
		to {
			opacity: 1;
		}
	}

	@keyframes ex-scrim-out {
		from {
			opacity: 1;
		}
		to {
			opacity: 0;
		}
	}

	@media (prefers-reduced-motion: reduce) {
		.ex-card,
		.ex-scrim,
		.ex-root.is-closing .ex-card,
		.ex-root.is-closing .ex-scrim {
			animation-duration: 0.01ms;
		}

		.ex-item {
			transition: none;
		}
	}
</style>
