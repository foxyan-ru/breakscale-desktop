<script module lang="ts">
	interface Row {
		/** Chords, each rendered as one key cap; alternatives joined with "or". */
		keys: string[];
		does: string;
	}

	interface Group {
		title: string;
		rows: Row[];
	}

	/**
	 * The reference list itself.
	 *
	 * Ported verbatim from src/components/Shortcuts.tsx. The React app wires
	 * the actual key bindings this table describes in App.tsx's keydown
	 * handlers, not in the Shortcuts component -- Shortcuts.tsx only ever
	 * displays this table. The same split holds here: this component's job is
	 * to show the reference faithfully, so it is ported completely.
	 *
	 * NOTE(integration): this component only ever DISPLAYS the table, exactly
	 * as Shortcuts.tsx does -- the handlers live in the root components,
	 * mirroring App.tsx (canvas keydown for selection/clipboard/zoom, the
	 * root page's keydown for Space, S, Ctrl+/ and the toggles). The
	 * "nothing is live" caveat that used to sit here went stale when those
	 * handlers landed, but the component's own contract has not changed: it
	 * is a fixed, non-empty, compile-time table, so there is no loading or
	 * empty state to model, and nothing here reads from Rust.
	 */
	const GROUPS: Group[] = [
		{
			title: 'Simulation',
			rows: [
				{ keys: ['Space'], does: 'Play or pause' },
				{ keys: ['S'], does: 'Step one tick (pauses first)' },
			],
		},
		{
			title: 'Build',
			rows: [
				{ keys: ['Drag a port'], does: 'Connect two components' },
				{ keys: ['Click a port'], does: 'Arm a link; click the target to finish' },
				{ keys: ['Double-click'], does: 'Rename a component in place' },
				{ keys: ['Ctrl+D'], does: 'Duplicate the selection' },
				{ keys: ['Alt+drag'], does: 'Drag out a copy' },
				{ keys: ['Delete', 'Backspace'], does: 'Delete the selection' },
			],
		},
		{
			title: 'Clipboard and history',
			rows: [
				{ keys: ['Ctrl+C'], does: 'Copy the selection' },
				{ keys: ['Ctrl+X'], does: 'Cut the selection' },
				{ keys: ['Ctrl+V'], does: 'Paste at the pointer' },
				{ keys: ['Ctrl+Z'], does: 'Undo' },
				{ keys: ['Ctrl+Shift+Z', 'Ctrl+Y'], does: 'Redo' },
			],
		},
		{
			title: 'Select and move',
			rows: [
				{ keys: ['Shift+click'], does: 'Add to or remove from the selection' },
				{ keys: ['Shift+drag'], does: 'Box-select on empty space' },
				{ keys: ['Ctrl+A'], does: 'Select everything' },
				{ keys: ['Arrows'], does: 'Move the selection one grid step' },
				{ keys: ['Shift+Arrows'], does: 'Move by 1px, off the grid' },
				{ keys: ['G'], does: 'Snap to the grid, on or off' },
				{ keys: ['Ctrl while dragging'], does: 'Bypass the snap for one drag' },
				{ keys: ['Esc'], does: 'Cancel the gesture and deselect' },
			],
		},
		{
			title: 'Notes and sections',
			rows: [
				{ keys: ['N'], does: 'Note tool: click the canvas to place text' },
				{ keys: ['B'], does: 'Section tool: drag to frame a group' },
				{ keys: ['Double-click a note'], does: 'Edit its text in place' },
				{ keys: ['Double-click a section'], does: 'Rename its label' },
			],
		},
		{
			title: 'View',
			rows: [
				{ keys: ['Ctrl+=', 'Ctrl+-'], does: 'Zoom in or out' },
				{ keys: ['Ctrl+0'], does: 'Zoom to 100%' },
				{ keys: ['Shift+1'], does: 'Zoom to fit everything' },
				{ keys: ['Shift+2'], does: 'Zoom to the selection' },
				{ keys: ['Ctrl+wheel'], does: 'Zoom at the cursor' },
				{ keys: ['Space+drag', 'Middle-drag'], does: 'Pan' },
			],
		},
		{
			title: 'Panels and help',
			rows: [
				{ keys: ['C'], does: 'Components rail' },
				{ keys: ['M'], does: 'Charts strip' },
				{ keys: ['I'], does: 'Inspector, while something is selected' },
				{ keys: ['?'], does: 'Glossary' },
				{ keys: ['Ctrl+/'], does: 'This dialog' },
			],
		},
	];
</script>

<script lang="ts">
	/* ==========================================================================
	   The keyboard shortcuts dialog.

	   Svelte port of src/components/Shortcuts.tsx. See that file's header for
	   the full rationale (a compact centred card, grouped by task not
	   alphabetically, Ctrl+/ rather than stealing "?").
	   ========================================================================== */

	interface Props {
		open: boolean;
		onClose: () => void;
	}

	let { open, onClose }: Props = $props();

	let cardEl: HTMLDivElement | undefined = $state(undefined);

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

	/* Focus lands on the card at open and returns to the opener at close
	   start. The card itself is the focus target (tabindex -1): its only
	   control is the close button, and landing there first would announce
	   "close" before the title. */
	$effect(() => {
		if (!open) return;
		const opener =
			document.activeElement instanceof HTMLElement ? document.activeElement : null;
		const card = cardEl;
		card?.focus();
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
			return;
		}
		/* Focus trap. The dialog holds exactly one focusable control, so the
		   trap is simply "Tab goes to it, and from it back". */
		if (e.key === 'Tab') {
			e.preventDefault();
			const btn = cardEl?.querySelector<HTMLElement>('button');
			if (document.activeElement === btn) cardEl?.focus();
			else btn?.focus();
		}
	}
</script>

{#if mounted}
	<div class={`sc-root${closing ? ' is-closing' : ''}`} inert={closing || undefined}>
		<div class="sc-scrim" onclick={onClose} aria-hidden="true"></div>
		<div
			bind:this={cardEl}
			class="sc-card"
			role="dialog"
			aria-modal="true"
			aria-labelledby="sc-title"
			tabindex="-1"
			onkeydown={onKeyDown}
			onanimationend={handleAnimationEnd}
		>
			<header class="sc-head">
				<h2 id="sc-title" class="sc-title">Keyboard shortcuts</h2>
				<button
					type="button"
					class="btn btn-ghost btn-sm btn-icon"
					onclick={onClose}
					aria-label="Close keyboard shortcuts"
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
			</header>

			<div class="sc-body">
				{#each GROUPS as g (g.title)}
					<section class="sc-group">
						<h3 class="sc-group-title">{g.title}</h3>
						<dl class="sc-rows">
							{#each g.rows as row (row.does)}
								<div class="sc-row">
									<dt class="sc-keys">
										{#each row.keys as k, i (k)}
											<span class="sc-alt">
												{#if i > 0}<span class="sc-or">or</span>{/if}
												<kbd class="sc-kbd">{k}</kbd>
											</span>
										{/each}
									</dt>
									<dd class="sc-does">{row.does}</dd>
								</div>
							{/each}
						</dl>
					</section>
				{/each}
			</div>
		</div>
	</div>
{/if}

<style>
	/* ==========================================================================
	   Keyboard shortcuts dialog. Ported from Shortcuts.css.

	   This file owns .sc-* only. It reuses .btn and the tokens from app.css
	   and re-declares none of them.
	   ========================================================================== */

	.sc-root {
		position: fixed;
		inset: 0;
		z-index: 500;
		display: grid;
		place-items: center;
		padding: var(--sp-5);
	}

	.sc-scrim {
		position: absolute;
		inset: 0;
		background: var(--scrim);
		cursor: default;
	}

	.sc-card {
		position: relative;
		display: flex;
		flex-direction: column;
		width: min(720px, 100%);
		max-height: min(640px, 100%);
		min-height: 0;
		background: var(--surface);
		border: var(--bw) solid var(--border-strong);
		border-radius: var(--r-lg);
		box-shadow: var(--shadow-lg);
		outline: none;
		animation: sc-card-in var(--dur-slow) var(--ease-out);
	}

	.sc-root.is-closing .sc-card {
		animation: sc-card-out var(--dur-base) var(--ease) forwards;
	}

	.sc-root.is-closing .sc-scrim {
		animation: sc-scrim-out var(--dur-base) var(--ease) forwards;
	}

	.sc-scrim {
		animation: sc-scrim-in var(--dur-slow) var(--ease-out);
	}

	@keyframes sc-card-in {
		from {
			opacity: 0;
			transform: translateY(6px);
		}
		to {
			opacity: 1;
			transform: translateY(0);
		}
	}

	@keyframes sc-card-out {
		from {
			opacity: 1;
			transform: translateY(0);
		}
		to {
			opacity: 0;
			transform: translateY(4px);
		}
	}

	@keyframes sc-scrim-in {
		from {
			opacity: 0;
		}
		to {
			opacity: 1;
		}
	}

	@keyframes sc-scrim-out {
		from {
			opacity: 1;
		}
		to {
			opacity: 0;
		}
	}

	.sc-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--sp-3);
		padding: var(--sp-4) var(--sp-5) var(--sp-3);
		border-bottom: var(--bw) solid var(--border);
	}

	.sc-title {
		margin: 0;
		font-size: var(--fs-base);
		font-weight: var(--fw-med);
		letter-spacing: var(--tr-base);
		color: var(--text);
	}

	.sc-body {
		min-height: 0;
		overflow-y: auto;
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: var(--sp-4) var(--sp-7);
		padding: var(--sp-4) var(--sp-5) var(--sp-5);
	}

	@media (max-width: 640px) {
		.sc-body {
			grid-template-columns: 1fr;
		}
	}

	.sc-group-title {
		margin: 0 0 var(--sp-2);
		font-size: var(--fs-label);
		font-weight: var(--fw-num);
		letter-spacing: var(--tr-label);
		text-transform: uppercase;
		color: var(--text-faint);
	}

	.sc-rows {
		margin: 0;
		display: flex;
		flex-direction: column;
		gap: var(--sp-1);
	}

	.sc-row {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		gap: var(--sp-3);
	}

	.sc-keys {
		display: flex;
		align-items: baseline;
		gap: var(--sp-1);
		flex: none;
	}

	.sc-alt {
		display: inline-flex;
		align-items: baseline;
		gap: var(--sp-1);
	}

	.sc-or {
		font-size: var(--fs-label);
		color: var(--text-faint);
	}

	.sc-kbd {
		padding: 1px var(--sp-1);
		border: var(--bw) solid var(--border-strong);
		border-radius: var(--r-mark);
		background: var(--surface-2);
		color: var(--text-dim);
		font-family: var(--mono);
		font-size: var(--fs-label);
		font-weight: var(--fw-med);
		font-variant-numeric: tabular-nums;
		line-height: var(--lh-label);
		white-space: nowrap;
	}

	.sc-does {
		margin: 0;
		font-size: var(--fs-sm);
		line-height: var(--lh-sm);
		color: var(--text-dim);
		text-align: right;
	}

	@media (prefers-reduced-motion: reduce) {
		.sc-card,
		.sc-scrim,
		.sc-root.is-closing .sc-card,
		.sc-root.is-closing .sc-scrim {
			animation-duration: 0.01ms;
		}
	}
</style>
