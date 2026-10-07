<script module lang="ts">
	/**
	 * One row in the main menu. Ported from `MenuItem` in
	 * src/components/MainMenu.tsx.
	 */
	export interface MenuItem {
		label: string;
		/** Path data for a 24-box stroked icon, matching the rest of the app. */
		icon: string;
		/** Printed on the right, so a binding is learnable from the menu itself. */
		hint?: string;
		onSelect: () => void;
	}
</script>

<script lang="ts">
	/* ==========================================================================
	   The main menu.

	   Svelte port of src/components/MainMenu.tsx. This component is
	   deliberately generic -- it renders whatever `items` it is given -- the
	   same way the React original did: App.tsx builds the actual item list
	   (Your designs, Challenges, Examples, Glossary, Keyboard shortcuts,
	   Settings, with icons/hints/handlers) and passes it down as a prop. The
	   root layout for this port is expected to do the same, once those panels
	   are wired together; this file owns the menu's own chrome, not its
	   contents.

	   WHY IT EXISTS. Reference-and-setup actions reached BETWEEN actions
	   rather than during one, folded behind a single button so the bar only
	   carries what changes while the simulation runs.

	   A menu, not a dialog: it opens beside its button, closes on the next
	   click anywhere, and never covers the canvas.
	   ========================================================================== */

	interface Props {
		open: boolean;
		onClose: () => void;
		items: MenuItem[];
	}

	let { open, onClose, items }: Props = $props();

	let listEl: HTMLDivElement | undefined = $state(undefined);

	/* Presence: kept in the DOM while the exit animation runs, then removed.
	   See Glossary.svelte for the fuller explanation of this pattern. */
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

	/*
	 * Close on a press anywhere else, and on Escape.
	 *
	 * Pointerdown rather than click, so the menu is gone before whatever was
	 * pressed reacts; a canvas gesture starting under an open menu that only
	 * closes on click would run with the menu still painted over it.
	 */
	$effect(() => {
		if (!open) return;
		const onDown = (e: PointerEvent) => {
			const el = listEl;
			if (!el || !(e.target instanceof Node) || el.contains(e.target)) return;
			// A press on the button that OPENED this menu is left alone, so its
			// own click handler can toggle rather than fighting this one.
			if (e.target instanceof Element && e.target.closest('[aria-haspopup="menu"]')) {
				return;
			}
			onClose();
		};
		const onKey = (e: KeyboardEvent) => {
			if (e.key === 'Escape') {
				e.stopPropagation();
				onClose();
			}
		};
		// Deferred a frame: the pointerdown that OPENED the menu would
		// otherwise be the one that closes it.
		const t = window.setTimeout(() => {
			document.addEventListener('pointerdown', onDown);
			document.addEventListener('keydown', onKey);
		}, 0);
		return () => {
			window.clearTimeout(t);
			document.removeEventListener('pointerdown', onDown);
			document.removeEventListener('keydown', onKey);
		};
	});

	/* Focus the first item on open and return focus to the opener on close. */
	$effect(() => {
		if (!open) return;
		const opener =
			document.activeElement instanceof HTMLElement ? document.activeElement : null;
		const list = listEl;
		list?.querySelector('button')?.focus();
		return () => {
			const active = document.activeElement;
			if (!active || active === document.body || list?.contains(active)) {
				opener?.focus();
			}
		};
	});

	/** Arrow keys walk the list, which is what a menu role promises. */
	function onKeyDown(e: KeyboardEvent) {
		if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
		e.preventDefault();
		const buttons = Array.from(listEl?.querySelectorAll('button') ?? []) as HTMLButtonElement[];
		if (buttons.length === 0) return;
		const i = buttons.indexOf(document.activeElement as HTMLButtonElement);
		const next =
			e.key === 'ArrowDown'
				? buttons[(i + 1 + buttons.length) % buttons.length]
				: buttons[(i - 1 + buttons.length) % buttons.length];
		next?.focus();
	}
</script>

{#if mounted}
	<div
		bind:this={listEl}
		class={`mn${closing ? ' is-closing' : ''}`}
		role="menu"
		aria-label="Menu"
		onkeydown={onKeyDown}
		onanimationend={handleAnimationEnd}
	>
		{#each items as item (item.label)}
			<button
				type="button"
				role="menuitem"
				class="mn-item"
				onclick={() => {
					// Closed BEFORE the action runs, so a panel the action opens is
					// not competing with a menu still on screen.
					onClose();
					item.onSelect();
				}}
			>
				<svg
					width="16"
					height="16"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="2"
					stroke-linecap="round"
					stroke-linejoin="round"
					aria-hidden="true"
				>
					<path d={item.icon} />
				</svg>
				<span class="mn-label">{item.label}</span>
				{#if item.hint}
					<kbd class="mn-hint" aria-hidden="true">{item.hint}</kbd>
				{/if}
			</button>
		{/each}
	</div>
{/if}

<style>
	/* ==========================================================================
	   Main menu. Ported from MainMenu.css.

	   Anchored under its button in the top bar rather than centred like a
	   dialog, because it belongs to that button.
	   ========================================================================== */

	.mn {
		position: absolute;
		top: calc(100% + var(--sp-2));
		right: 0;
		z-index: 200;
		min-width: 15rem;
		padding: var(--sp-1);
		display: flex;
		flex-direction: column;
		gap: 1px;
		background: var(--surface);
		border: var(--bw) solid var(--border-strong);
		border-radius: var(--r-md);
		box-shadow: var(--shadow-lg);
		transform-origin: top right;
		animation: mn-in var(--dur-base) var(--ease-out);
	}

	.mn.is-closing {
		animation: mn-out var(--dur-fast) var(--ease) forwards;
	}

	@keyframes mn-in {
		from {
			opacity: 0;
			transform: translateY(-6px) scale(0.97);
		}
		to {
			opacity: 1;
			transform: translateY(0) scale(1);
		}
	}

	@keyframes mn-out {
		from {
			opacity: 1;
			transform: translateY(0);
		}
		to {
			opacity: 0;
			transform: translateY(-4px);
		}
	}

	.mn-item {
		display: flex;
		align-items: center;
		gap: var(--sp-3);
		width: 100%;
		height: 34px;
		padding: 0 var(--sp-2);
		border: none;
		border-radius: var(--r-sm);
		background: none;
		color: var(--text);
		font: inherit;
		font-size: var(--fs-base);
		text-align: left;
		cursor: pointer;
	}

	.mn-item:hover {
		background: var(--surface-2);
	}

	.mn-item:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: -2px;
	}

	.mn-item svg {
		flex: none;
		color: var(--text-dim);
	}

	.mn-item:hover svg {
		color: var(--accent);
	}

	.mn-label {
		flex: 1 1 auto;
		min-width: 0;
	}

	.mn-hint {
		flex: none;
		font-family: var(--mono);
		font-size: var(--fs-label);
		color: var(--text-faint);
	}

	@media (prefers-reduced-motion: reduce) {
		.mn,
		.mn.is-closing {
			animation-duration: 0.01ms;
		}
	}
</style>
