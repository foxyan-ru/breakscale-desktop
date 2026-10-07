<script module lang="ts">
	/**
	 * Glossary entry shape.
	 *
	 * Mirrors `GlossaryEntry` from the web app's `src/content/glossary.ts`
	 * (990 lines of plain-language definitions). A parallel agent ported that
	 * content to `src-tauri/data/glossary.json` plus a Rust loader,
	 * but nothing exposes it over IPC yet.
	 *
	 * NOTE(integration): needs a `glossary_list` Tauri command backed by
	 * `sim::glossary::load()` (or whatever the loader ends up being named),
	 * plus a wrapper in `desktop/src/lib/api/*.ts`. Until that exists, this
	 * component takes its entries as a prop -- see `Props.entries` below --
	 * so it is fully testable/reviewable now. The type is exported from this
	 * module (not a domain file, since none exists for it yet) so the root
	 * layout and the future API wrapper can both import it without drift.
	 */
	export type GlossaryCategory =
		| 'latency'
		| 'throughput'
		| 'failure'
		| 'capacity'
		| 'component'
		| 'unit';

	export interface GlossaryEntry {
		/** Stable lookup key. Also the DOM anchor id for this entry. */
		id: string;
		/** How the term appears in the interface. */
		term: string;
		/** One-line definition. Tooltip header; no trailing period. */
		short: string;
		/** Why a student should care. Two or three sentences at most. */
		why: string;
		category: GlossaryCategory;
		/** Ids of related entries, for "see also" links. */
		see?: string[];
		/** Extra words that should match this entry when searching. */
		aliases?: string[];
	}

	/** Order the category sections appear in. Units first: they are the
	 *  smallest ideas and everything else is described using them. */
	const CATEGORY_ORDER: GlossaryCategory[] = [
		'unit',
		'latency',
		'throughput',
		'capacity',
		'failure',
		'component',
	];

	const CATEGORY_LABEL: Record<GlossaryCategory, string> = {
		latency: 'Latency',
		throughput: 'Throughput',
		failure: 'Failures',
		capacity: 'Capacity',
		component: 'Components',
		unit: 'Units',
	};

	interface Section {
		category: GlossaryCategory;
		entries: GlossaryEntry[];
	}

	/** Groups a flat result list into category sections, preserving rank order. */
	function group(entries: GlossaryEntry[]): Section[] {
		const byCategory = new Map<GlossaryCategory, GlossaryEntry[]>();
		for (const e of entries) {
			const list = byCategory.get(e.category);
			if (list) list.push(e);
			else byCategory.set(e.category, [e]);
		}
		const out: Section[] = [];
		for (const category of CATEGORY_ORDER) {
			const list = byCategory.get(category);
			if (list && list.length > 0) out.push({ category, entries: list });
		}
		return out;
	}

	/**
	 * Search across term, aliases and definition text.
	 *
	 * Ranked so an exact term match always beats an incidental mention in
	 * some other entry's `why` text. Ported verbatim from `searchGlossary` in
	 * `src/content/glossary.ts`.
	 */
	function search(all: GlossaryEntry[], query: string): GlossaryEntry[] {
		const q = query.trim().toLowerCase();
		if (!q) return all;

		const scored: Array<{ entry: GlossaryEntry; score: number }> = [];
		for (const entry of all) {
			const term = entry.term.toLowerCase();
			const aliases = entry.aliases?.map((a) => a.toLowerCase()) ?? [];

			let score = 0;
			if (term === q || aliases.includes(q)) score = 100;
			else if (term.startsWith(q)) score = 80;
			else if (aliases.some((a) => a.startsWith(q))) score = 70;
			else if (term.includes(q)) score = 50;
			else if (aliases.some((a) => a.includes(q))) score = 40;
			else if (entry.short.toLowerCase().includes(q)) score = 20;
			else if (entry.why.toLowerCase().includes(q)) score = 10;

			if (score > 0) scored.push({ entry, score });
		}

		scored.sort((a, b) =>
			b.score !== a.score ? b.score - a.score : a.entry.term.localeCompare(b.entry.term),
		);
		return scored.map((s) => s.entry);
	}
</script>

<script lang="ts">
	/* ==========================================================================
	   The glossary panel.

	   Svelte port of src/components/Glossary.tsx. See that file's header for
	   the full rationale (side sheet vs modal, why it exists alongside the
	   hover tooltips); it applies unchanged here.

	   WHY A SHEET, NOT A MODAL. A student opens this WHILE the simulation is
	   running, precisely because a number on screen confused them. The sheet
	   takes the right edge, over the least costly region to lose, and the
	   canvas stays visible and live the whole time it is open.

	   THE PAGE NEVER SCROLLS. The sheet is fixed to the viewport and its
	   entry list is the only thing that scrolls, inside itself.
	   ========================================================================== */

	interface Props {
		open: boolean;
		/** Called on Escape, scrim click, or the close button. */
		onClose: () => void;
		/** Every glossary entry. See the NOTE(integration) above module script. */
		entries: GlossaryEntry[];
		/** True while `entries` is still being fetched from Rust. */
		loading?: boolean;
		/**
		 * Scroll to and highlight this entry on open. This is where a
		 * tooltip's "see also" link lands, and where the shell should send a
		 * student who asked about a specific term.
		 */
		focusId?: string;
	}

	let { open, onClose, entries, loading = false, focusId }: Props = $props();

	let query = $state('');
	/** The entry the arrow keys are currently on. */
	let activeId = $state<string | null>(null);
	/** The entry that was jumped to, briefly emphasised so the eye finds it. */
	let landedId = $state<string | null>(null);

		let sheetEl: HTMLDivElement | undefined = $state(undefined);
	let searchEl: HTMLInputElement | undefined = $state(undefined);
	let listEl: HTMLDivElement | undefined = $state(undefined);

	/* Presence: kept in the DOM while the exit animation in Glossary's style
	   block runs, then removed. Ported inline from the React app's
	   usePresence hook (src/components/presence.tsx) -- only the files listed
	   in this task's Build section may be created, so no shared helper. */
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

	function handleSheetAnimationEnd(e: AnimationEvent) {
		if (closing && e.target === e.currentTarget) {
			mounted = false;
			closing = false;
		}
	}

	const entriesById = $derived(new Map(entries.map((e) => [e.id, e])));
	const results = $derived(search(entries, query));
	const sections = $derived(group(results));
	/** Flat, in the order the sections actually render, for arrow navigation. */
	const ordered = $derived(sections.flatMap((s) => s.entries));

	/* ---------------------------------------------------------------- *
	 * Jumping to an entry
	 * ---------------------------------------------------------------- */

	/**
	 * Reveals an entry: clears any filter hiding it, moves the selection to
	 * it, scrolls it into view and marks it as landed. Clearing the query
	 * matters -- a cross-reference is useless if the search box still filters
	 * the target out of the list the moment it is asked for.
	 */
	function jumpTo(id: string) {
		if (!entriesById.has(id)) return;
		query = '';
		activeId = id;
		landedId = id;
	}

	/* Scroll the active entry into view whenever it changes. */
	$effect(() => {
		if (!open || !activeId) return;
		ordered; // re-run when the visible order changes too
		const el = listEl?.querySelector<HTMLElement>(`[data-entry="${CSS.escape(activeId)}"]`);
		el?.scrollIntoView({ block: 'nearest' });
	});

	/* The landed emphasis is a one-shot cue, not a persistent state. */
	$effect(() => {
		if (!landedId) return;
		const t = window.setTimeout(() => {
			landedId = null;
		}, 1600);
		return () => window.clearTimeout(t);
	});

	/* ---------------------------------------------------------------- *
	 * Open / close lifecycle
	 * ---------------------------------------------------------------- */

	$effect(() => {
		if (!open) return;

		const opener =
			document.activeElement instanceof HTMLElement ? document.activeElement : null;

		// Focus the search box: the panel's primary job is lookup.
		searchEl?.focus();

		const sheet = sheetEl;
		return () => {
			const active = document.activeElement;
			const inside = sheet?.contains(active as Node) ?? false;
			if (inside || active === document.body || active === null) {
				opener?.focus();
			}
		};
	});

	/* Reset between visits, so reopening does not resume a stale search. */
	$effect(() => {
		if (!open) return;
		query = '';
		activeId = null;
		landedId = null;
	});

	/* Opening with a target, or being handed a new one while already open. */
	$effect(() => {
		if (!open || !focusId) return;
		jumpTo(focusId);
	});

	/* ---------------------------------------------------------------- *
	 * Keyboard
	 * ---------------------------------------------------------------- */

	/**
	 * Moves the selection by `delta` entries, clamped at both ends rather
	 * than wrapping -- wrapping in a long list disorients more than it helps.
	 */
	function move(delta: number) {
		if (ordered.length === 0) return;
		const at = activeId ? ordered.findIndex((e) => e.id === activeId) : -1;
		const next =
			at < 0
				? delta > 0
					? 0
					: ordered.length - 1
				: Math.min(Math.max(at + delta, 0), ordered.length - 1);
		activeId = ordered[next]?.id ?? null;
	}

	function onKeyDown(e: KeyboardEvent) {
		switch (e.key) {
			case 'Escape':
				e.preventDefault();
				e.stopPropagation();
				onClose();
				return;
			case 'ArrowDown':
				e.preventDefault();
				move(1);
				return;
			case 'ArrowUp':
				e.preventDefault();
				move(-1);
				return;
			case 'Home':
				if (e.target === searchEl) return;
				e.preventDefault();
				activeId = ordered[0]?.id ?? null;
				return;
			case 'End':
				if (e.target === searchEl) return;
				e.preventDefault();
				activeId = ordered[ordered.length - 1]?.id ?? null;
				return;
			case 'Enter': {
				// Enter from the search box commits to the top hit.
				if (e.target === searchEl) {
					const first = ordered[0];
					if (!first) return;
					e.preventDefault();
					jumpTo(first.id);
				}
				return;
			}
			case 'Tab': {
				/* Focus trap. Computed live, because the list changes as you type. */
				const sheet = sheetEl;
				if (!sheet) return;
				const focusables = sheet.querySelectorAll<HTMLElement>(
					'button:not([disabled]), input, [href], select, textarea, [tabindex]:not([tabindex="-1"])',
				);
				if (focusables.length === 0) return;
				const first = focusables[0];
				const last = focusables[focusables.length - 1];
				if (!first || !last) return;
				const active = document.activeElement;
				if (e.shiftKey && active === first) {
					e.preventDefault();
					last.focus();
				} else if (!e.shiftKey && active === last) {
					e.preventDefault();
					first.focus();
				}
				return;
			}
			default:
				return;
		}
	}

	function onSearchInput(e: Event) {
		query = (e.currentTarget as HTMLInputElement).value;
		// The old selection is probably filtered out now; leaving it set would
		// make the next arrow press jump somewhere random.
		activeId = null;
	}

	function clearSearch() {
		query = '';
		activeId = null;
		searchEl?.focus();
	}
</script>

{#if mounted}
	<div class={`gl-root${closing ? ' is-closing' : ''}`} inert={closing || undefined}>
		<!-- The scrim marks the sheet as the active layer; it carries no
		     information, so it is not announced. -->
		<div class="gl-scrim" onclick={onClose} aria-hidden="true"></div>

		<div
			bind:this={sheetEl}
			class="gl-sheet"
			role="dialog"
			aria-modal="true"
			aria-labelledby="gl-title"
			onkeydown={onKeyDown}
			onanimationend={handleSheetAnimationEnd}
		>
			<header class="gl-head">
				<div class="gl-head-row">
					<h2 id="gl-title" class="gl-title">Glossary</h2>
					<button
						type="button"
						class="btn btn-ghost btn-sm btn-icon"
						onclick={onClose}
						aria-label="Close glossary"
					>
						<svg
							width="16"
							height="16"
							viewBox="0 0 24 24"
							fill="none"
							stroke="currentColor"
							stroke-width="2"
							stroke-linecap="round"
							aria-hidden="true"
						>
							<path d="M18 6 6 18M6 6l12 12" />
						</svg>
					</button>
				</div>

				<p class="gl-sub">Every term this app puts on screen, and why it matters.</p>

				<div class="gl-search">
					<svg
						class="gl-search-icon"
						width="16"
						height="16"
						viewBox="0 0 24 24"
						fill="none"
						stroke="currentColor"
						stroke-width="2"
						stroke-linecap="round"
						aria-hidden="true"
					>
						<circle cx="11" cy="11" r="7" />
						<path d="m20 20-3.5-3.5" />
					</svg>
					<input
						bind:this={searchEl}
						type="text"
						class="gl-search-input"
						placeholder="Search terms"
						value={query}
						oninput={onSearchInput}
						aria-label="Search glossary terms"
						aria-describedby="gl-count"
						autocomplete="off"
						spellcheck="false"
					/>
					{#if query}
						<button
							type="button"
							class="gl-search-clear"
							onclick={clearSearch}
							aria-label="Clear search"
						>
							<svg
								width="14"
								height="14"
								viewBox="0 0 24 24"
								fill="none"
								stroke="currentColor"
								stroke-width="2"
								stroke-linecap="round"
								aria-hidden="true"
							>
								<path d="M18 6 6 18M6 6l12 12" />
							</svg>
						</button>
					{/if}
				</div>

				<!-- aria-live so a screen reader hears the result count change. -->
				<p id="gl-count" class="gl-count" aria-live="polite">
					{#if loading}
						Loading termsâ€¦
					{:else if query}
						{results.length} of {entries.length}
						{results.length === 1 ? 'term' : 'terms'}
					{:else}
						{entries.length} terms
					{/if}
				</p>
			</header>

			<div bind:this={listEl} class="gl-list scroll">
				{#if loading}
					<div class="empty">
						<p>Loading the glossaryâ€¦</p>
					</div>
				{:else if results.length === 0}
					<div class="empty">
						<p>No term matches "{query}".</p>
						<p>Try a shorter word, or clear the search to browse them all.</p>
					</div>
				{:else}
					{#each sections as section (section.category)}
						<section class="gl-section">
							<h3 class="label gl-section-title">{CATEGORY_LABEL[section.category]}</h3>
							<ul class="gl-entries">
								{#each section.entries as entry (entry.id)}
									{@const see = (entry.see ?? [])
										.map((id) => entriesById.get(id))
										.filter((e): e is GlossaryEntry => e !== undefined)}
									<li
										data-entry={entry.id}
										class={`gl-entry${entry.id === activeId ? ' is-active' : ''}${entry.id === landedId ? ' is-landed' : ''}`}
										onclick={() => (activeId = entry.id)}
									>
										<h4 class="gl-term">{entry.term}</h4>
										<p class="gl-short">{entry.short}</p>
										<p class="gl-why">{entry.why}</p>

										{#if see.length > 0}
											<p class="gl-see">
												<span class="gl-see-label">See also</span>
												{#each see as other (other.id)}
													<button
														type="button"
														class="gl-see-link"
														onclick={(e) => {
															e.stopPropagation();
															jumpTo(other.id);
														}}
													>
														{other.term}
													</button>
												{/each}
											</p>
										{/if}
									</li>
								{/each}
							</ul>
						</section>
					{/each}
				{/if}
			</div>
		</div>
	</div>
{/if}

<style>
	/* ==========================================================================
	   Glossary â€” the browsable reference. Ported from Glossary.css.

	   A right-hand side sheet, fixed to the viewport. See the header comment
	   in the script block above for why a sheet and not a modal.

	   THE PAGE NEVER SCROLLS. Everything here is position:fixed and the entry
	   list is the only scroll container.

	   This file owns .gl-* only. It reuses .btn, .label, .empty and .scroll
	   from app.css and re-declares none of them.
	   ========================================================================== */

	.gl-root {
		position: fixed;
		inset: 0;
		/* Above the tooltip layer (400): a tooltip must never float over the
		   sheet that has just covered its trigger. */
		z-index: 500;
	}

	.gl-scrim {
		position: absolute;
		inset: 0;
		background: var(--scrim);
		cursor: default;
	}

	.gl-sheet {
		position: absolute;
		top: 0;
		right: 0;
		bottom: 0;
		display: flex;
		flex-direction: column;
		width: min(400px, 100vw);
		min-height: 0;
		background: var(--surface);
		border-left: var(--bw) solid var(--border);
		box-shadow: var(--shadow-lg);
	}

	@keyframes gl-sheet-in {
		from {
			transform: translateX(100%);
		}
	}

	@keyframes gl-sheet-out {
		to {
			transform: translateX(100%);
		}
	}

	@keyframes gl-scrim-in {
		from {
			opacity: 0;
		}
	}

	@keyframes gl-scrim-out {
		to {
			opacity: 0;
		}
	}

	.gl-sheet {
		animation: gl-sheet-in var(--dur-slow) var(--ease-out);
	}

	.gl-scrim {
		animation: gl-scrim-in var(--dur-slow) var(--ease-out);
	}

	.gl-root.is-closing .gl-sheet {
		animation: gl-sheet-out var(--dur-base) var(--ease) forwards;
	}

	.gl-root.is-closing .gl-scrim {
		animation: gl-scrim-out var(--dur-base) var(--ease) forwards;
	}

	.gl-head {
		flex: none;
		padding: var(--sp-4) var(--sp-5) var(--sp-3);
		border-bottom: var(--bw) solid var(--border);
	}

	.gl-head-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--sp-3);
	}

	.gl-title {
		margin: 0;
		font-size: var(--fs-num);
		font-weight: var(--fw-num);
		line-height: var(--lh-label);
		letter-spacing: var(--tr-num);
		color: var(--text);
	}

	.gl-sub {
		margin: var(--sp-1) 0 0;
		font-size: var(--fs-sm);
		font-weight: var(--fw-body);
		line-height: var(--lh-sm);
		color: var(--text-dim);
		text-wrap: pretty;
	}

	.gl-search {
		position: relative;
		display: flex;
		align-items: center;
		margin-top: var(--sp-3);
	}

	.gl-search-icon {
		position: absolute;
		left: var(--sp-3);
		color: var(--text-faint);
		pointer-events: none;
	}

	/* THE SELECTOR IS input.gl-search-input, NOT .gl-search-input, AND THAT IS
	   LOAD-BEARING. app.css styles the field as input[type='text'], whose
	   specificity is (0,1,1); a lone class is (0,1,0) and loses. Qualifying
	   with the element name makes this (0,1,1) too, so this rule wins the tie
	   by source order instead of silently losing the cascade. */
	input.gl-search-input {
		width: 100%;
		padding-left: calc(var(--sp-3) + 16px + var(--sp-2));
		padding-right: var(--sp-7);
	}

	.gl-search-clear {
		position: absolute;
		right: var(--sp-1);
		display: flex;
		align-items: center;
		justify-content: center;
		width: 28px;
		height: 28px;
		padding: 0;
		border: 0;
		border-radius: var(--r-sm);
		background: transparent;
		color: var(--text-faint);
		cursor: pointer;
		transition:
			background-color var(--dur-fast) var(--ease),
			color var(--dur-fast) var(--ease);
	}

	.gl-search-clear:hover {
		background: var(--surface-3);
		color: var(--text);
	}

	.gl-search-clear:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: -2px;
	}

	.gl-count {
		margin: var(--sp-2) 0 0;
		font-family: var(--mono);
		font-variant-numeric: tabular-nums;
		font-feature-settings: 'tnum' 1;
		font-size: var(--fs-label);
		font-weight: var(--fw-body);
		line-height: var(--lh-label);
		letter-spacing: var(--tr-num);
		color: var(--text-faint);
	}

	.gl-list {
		flex: 1 1 auto;
		padding: var(--sp-3) 0 var(--sp-7);
		overflow-x: hidden;
		overscroll-behavior: contain;
	}

	.gl-section + .gl-section {
		margin-top: var(--sp-5);
	}

	.gl-section-title {
		position: sticky;
		top: 0;
		z-index: 1;
		padding: var(--sp-2) var(--sp-5);
		background: var(--surface);
	}

	.gl-entries {
		list-style: none;
		margin: 0;
		padding: 0;
	}

	.gl-entry {
		padding: var(--sp-3) var(--sp-5) var(--sp-3) calc(var(--sp-5) - 3px);
		border-left: 3px solid transparent;
		cursor: default;
		transition:
			background-color var(--dur-fast) var(--ease),
			border-left-color var(--dur-fast) var(--ease);
	}

	.gl-entry:hover {
		background: var(--surface-2);
	}

	.gl-entry.is-active {
		background: var(--accent-soft);
	}

	.gl-entry.is-landed {
		background: var(--accent-soft);
		border-left-color: var(--accent);
	}

	.gl-term {
		margin: 0;
		font-size: var(--fs-base);
		font-weight: var(--fw-num);
		line-height: var(--lh-label);
		letter-spacing: var(--tr-base);
		color: var(--text);
	}

	.gl-short {
		margin: var(--sp-1) 0 0;
		font-size: var(--fs-sm);
		font-weight: var(--fw-body);
		line-height: var(--lh-sm);
		color: var(--text-dim);
		text-wrap: pretty;
	}

	.gl-why {
		margin: var(--sp-2) 0 0;
		font-size: var(--fs-sm);
		font-weight: var(--fw-body);
		line-height: var(--lh-prose);
		color: var(--text);
		text-wrap: pretty;
	}

	.gl-see {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: var(--sp-1) var(--sp-2);
		margin: var(--sp-2) 0 0;
	}

	.gl-see-label {
		font-size: var(--fs-label);
		font-weight: var(--fw-med);
		line-height: var(--lh-label);
		letter-spacing: var(--tr-label);
		text-transform: uppercase;
		color: var(--text-faint);
	}

	.gl-see-link {
		padding: 0;
		border: 0;
		background: none;
		font-family: inherit;
		font-size: var(--fs-sm);
		font-weight: var(--fw-body);
		line-height: var(--lh-sm);
		color: var(--accent-ink);
		text-decoration: underline;
		text-underline-offset: 2px;
		border-radius: var(--r-sm);
		cursor: pointer;
		transition: color var(--dur-fast) var(--ease);
	}

	.gl-see-link:hover {
		color: var(--accent);
	}

	.gl-see-link:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 2px;
	}

	@media (max-width: 480px) {
		.gl-sheet {
			width: 100vw;
			border-left: none;
		}
	}

	@media (prefers-reduced-motion: reduce) {
		.gl-entry,
		.gl-search-clear,
		.gl-see-link {
			transition: none;
		}
	}
</style>
