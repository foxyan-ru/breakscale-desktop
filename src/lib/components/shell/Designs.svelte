<script lang="ts">
	/* ==========================================================================
	   Your designs.

	   Svelte port of src/components/Designs.tsx. See that file's header for
	   why this exists alongside autosave and file export (holding two ideas
	   at once) and why it is a dialog rather than a permanent rail.

	   UNLIKE THE REACT VERSION, this component is wired directly to the real
	   Tauri commands rather than taking onOpen/onSave callback props: the web
	   app kept designs in localStorage and read/wrote them synchronously, so
	   App.tsx could own that logic and hand this component two callbacks. On
	   desktop, saved designs live behind async IPC calls
	   (`desktop/src/lib/api/designs.ts`, backed by a JSON file in the OS
	   app-data directory -- MIGRATION_PLAN.md Â§7), and the topology itself
	   already lives in a shared store (`$lib/state/topology.svelte`) that any
	   component may drive directly. Calling the finished API/store modules
	   here -- rather than re-threading the same calls through callback props
	   -- keeps this panel self-contained and matches how `topology.svelte.ts`
	   itself expects to be used.

	   Adds file-based save/load (pickSavePath/pickOpenPath +
	   design_file_write/design_file_read) alongside the named-shelf
	   save/open the React version had, per this task's brief -- the original
	   pointed a reader at "Settings" for that; this port does it inline
	   since it already owns the dialog that talks to the design-file API.
	   ========================================================================== */

	import {
		designsList,
		designsSave,
		designsGet,
		designsDelete,
		designsRename,
		designFileWrite,
		designFileRead,
	} from '$lib/api/designs';
	import { pickOpenPath, pickSavePath } from '$lib/api/dialog';
	import { isAppError } from '$lib/api';
	import { pushError } from '$lib/state/ui.svelte';
	import { topologyStore, setTopology } from '$lib/state/topology.svelte';
	import { MAX_NAME, MAX_SAVED, DESIGN_FILE_EXT } from '$lib/domain';
	import type { SavedSummary } from '$lib/domain';

	interface Props {
		open: boolean;
		onClose: () => void;
		/** Suggested name for a new save, usually the loaded example's. */
		suggestedName?: string;
	}

	let { open, onClose, suggestedName = '' }: Props = $props();

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

	/* ---------------------------------------------------------------- *
	 * The shelf: designsList / designsSave / designsGet / designsDelete /
	 * designsRename, via desktop/src/lib/api/designs.ts.
	 * ---------------------------------------------------------------- */

	let items = $state<SavedSummary[]>([]);
	/** True until the first designsList() resolves (or fails). */
	let loadingList = $state(true);
	let listError = $state<string | null>(null);

	let name = $state('');
	let renaming = $state<string | null>(null);
	let saving = $state(false);
	let openingId = $state<string | null>(null);
	let fileBusy = $state(false);
	/**
	 * One inline error slot, shared by every action this dialog can take
	 * (save-to-shelf validation, rename conflicts, file-parse rejections).
	 * Mirrors the React version's single `dz-error` paragraph, which showed
	 * whatever the reader had just tried and failed at.
	 */
	let actionError = $state<string | null>(null);

	function describeError(action: string, e: unknown): string {
		const message = isAppError(e) ? e.message : e instanceof Error ? e.message : String(e);
		return `${action} failed: ${message}`;
	}

	async function refresh(): Promise<void> {
		loadingList = true;
		listError = null;
		try {
			items = await designsList();
		} catch (e) {
			listError = 'Could not load your saved designs.';
			pushError(describeError('Loading your designs', e));
		} finally {
			loadingList = false;
		}
	}

	/* Read the shelf on open, not on mount: something else may have saved
	   since, and this is the moment the reader is looking. */
	$effect(() => {
		if (!open) return;
		refresh();
		name = suggestedName;
		actionError = null;
		renaming = null;
	});

	$effect(() => {
		if (!open) return;
		const opener =
			document.activeElement instanceof HTMLElement ? document.activeElement : null;
		const card = cardEl;
		card?.focus();
		return () => {
			const active = document.activeElement;
			if (!active || active === document.body || card?.contains(active)) {
				opener?.focus();
			}
		};
	});

	async function doSave(): Promise<void> {
		const clean = name.trim();
		if (!clean) {
			actionError = 'Give the design a name first.';
			return;
		}
		saving = true;
		try {
			const result = await designsSave(clean, topologyStore.topology);
			if (result.ok) {
				actionError = null;
				await refresh();
			} else {
				actionError = result.error;
			}
		} catch (e) {
			const message = describeError('Saving the design', e);
			actionError = message;
			pushError(message);
		} finally {
			saving = false;
		}
	}

	async function openDesign(id: string): Promise<void> {
		openingId = id;
		try {
			const design = await designsGet(id);
			setTopology(design.topology);
			onClose();
		} catch (e) {
			pushError(describeError('Opening the design', e));
		} finally {
			openingId = null;
		}
	}

	async function commitRename(id: string, value: string): Promise<void> {
		const clean = value.trim();
		if (!clean) {
			renaming = null;
			return;
		}
		try {
			const ok = await designsRename(id, clean);
			if (!ok) {
				actionError = 'That name is already taken.';
			} else {
				actionError = null;
				await refresh();
			}
		} catch (e) {
			pushError(describeError('Renaming the design', e));
		} finally {
			renaming = null;
		}
	}

	async function deleteOne(id: string): Promise<void> {
		try {
			await designsDelete(id);
			await refresh();
		} catch (e) {
			pushError(describeError('Deleting the design', e));
		}
	}

	function onRenameKeyDown(id: string, e: KeyboardEvent) {
		e.stopPropagation();
		if (e.isComposing || e.keyCode === 229) return;
		if (e.key === 'Escape') {
			e.preventDefault();
			renaming = null;
		} else if (e.key === 'Enter') {
			e.preventDefault();
			void commitRename(id, (e.currentTarget as HTMLInputElement).value);
		}
	}

	/** Focuses an element the moment it is inserted -- used on the rename
	 *  input, which only exists while `renaming` names its row. */
	function focusOnMount(node: HTMLElement) {
		node.focus();
	}

	/* ---------------------------------------------------------------- *
	 * File-based save/load: pickSavePath/pickOpenPath (dialog.ts) +
	 * design_file_write/design_file_read (designs.ts). The named shelf above
	 * is quick and stays on this machine; a file is how a design travels.
	 * ---------------------------------------------------------------- */

	const FILE_FILTERS = [{ name: 'Breakscale design', extensions: ['breakscale'] }];

	function suggestedFileName(): string {
		const base = name.trim() || suggestedName.trim() || 'design';
		return `${base}${DESIGN_FILE_EXT}`;
	}

	async function exportToFile(): Promise<void> {
		fileBusy = true;
		try {
			const path = await pickSavePath(suggestedFileName(), FILE_FILTERS);
			if (!path) return;
			await designFileWrite(path, topologyStore.topology, name.trim() || suggestedName || null);
			actionError = null;
		} catch (e) {
			const message = describeError('Saving the design to a file', e);
			actionError = message;
			pushError(message);
		} finally {
			fileBusy = false;
		}
	}

	async function importFromFile(): Promise<void> {
		fileBusy = true;
		try {
			const path = await pickOpenPath(FILE_FILTERS);
			if (!path) return;
			const result = await designFileRead(path);
			if (result.ok) {
				setTopology(result.topology);
				actionError = null;
				onClose();
			} else {
				actionError = result.error;
			}
		} catch (e) {
			const message = describeError('Opening the design file', e);
			actionError = message;
			pushError(message);
		} finally {
			fileBusy = false;
		}
	}
</script>

{#if mounted}
	<div class={`dz-root${closing ? ' is-closing' : ''}`} inert={closing || undefined}>
		<div class="dz-scrim" onclick={onClose} aria-hidden="true"></div>
		<div
			bind:this={cardEl}
			class="dz-card"
			role="dialog"
			aria-modal="true"
			aria-labelledby="dz-title"
			tabindex="-1"
			onkeydown={(e) => {
				if (e.key === 'Escape') {
					e.preventDefault();
					e.stopPropagation();
					onClose();
				}
			}}
			onanimationend={handleAnimationEnd}
		>
			<header class="dz-head">
				<h2 id="dz-title" class="dz-title">Your designs</h2>
				<button
					type="button"
					class="btn btn-ghost btn-sm btn-icon"
					onclick={onClose}
					aria-label="Close your designs"
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

			<div class="dz-save">
				<input
					class="dz-name"
					type="text"
					value={name}
					maxlength={MAX_NAME}
					placeholder="Name this design"
					aria-label="Design name"
					oninput={(e) => (name = (e.currentTarget as HTMLInputElement).value)}
					onkeydown={(e) => {
						e.stopPropagation();
						if (e.isComposing || e.keyCode === 229) return;
						if (e.key === 'Enter') {
							e.preventDefault();
							void doSave();
						}
					}}
				/>
				<button
					type="button"
					class="btn btn-primary btn-sm"
					onclick={doSave}
					disabled={saving}
				>
					{saving ? 'Savingâ€¦' : 'Save'}
				</button>
			</div>

			<div class="dz-file-actions">
				<button
					type="button"
					class="btn btn-ghost btn-sm"
					onclick={importFromFile}
					disabled={fileBusy}
				>
					Open from fileâ€¦
				</button>
				<button
					type="button"
					class="btn btn-ghost btn-sm"
					onclick={exportToFile}
					disabled={fileBusy}
				>
					Save to fileâ€¦
				</button>
			</div>

			{#if actionError}
				<p class="dz-error" role="alert">{actionError}</p>
			{/if}

			<div class="dz-body">
				{#if loadingList}
					<p class="dz-empty">Loading your designsâ€¦</p>
				{:else if listError}
					<p class="dz-empty">
						{listError}
						<button type="button" class="btn btn-ghost btn-sm" onclick={refresh}>
							Try again
						</button>
					</p>
				{:else if items.length === 0}
					<p class="dz-empty">
						Nothing saved yet. Name what is on the canvas and press Save, and it will
						wait here for you.
					</p>
				{:else}
					<ul class="dz-list">
						{#each items as d (d.id)}
							<li class="dz-item">
								{#if renaming === d.id}
									<input
										class="dz-name dz-rename"
										type="text"
										value={d.name}
										maxlength={MAX_NAME}
										aria-label={`Rename ${d.name}`}
										use:focusOnMount
										onkeydown={(e) => onRenameKeyDown(d.id, e)}
										onblur={() => (renaming = null)}
									/>
								{:else}
									<button
										type="button"
										class="dz-open"
										disabled={openingId === d.id}
										onclick={() => openDesign(d.id)}
									>
										<span class="dz-item-name">{d.name}</span>
										<span class="dz-item-meta">
											{openingId === d.id
												? 'Openingâ€¦'
												: `${d.nodeCount} component${d.nodeCount === 1 ? '' : 's'}`}
										</span>
									</button>
								{/if}

								<div class="dz-actions">
									<button
										type="button"
										class="btn btn-ghost btn-sm"
										onclick={() => (renaming = d.id)}
									>
										Rename
									</button>
									<button
										type="button"
										class="btn btn-ghost btn-sm dz-delete"
										onclick={() => deleteOne(d.id)}
									>
										Delete
									</button>
								</div>
							</li>
						{/each}
					</ul>
				{/if}
			</div>

			<footer class="dz-foot">
				<!-- Said plainly: this is a local JSON file this app owns, not
				     something synced anywhere. A ".breakscale" file, saved above,
				     is how a design moves to another machine. -->
				Saved on this computer only, in Breakscale's own data folder. Nothing
				is uploaded. {items.length} of {MAX_SAVED} used. To move a design elsewhere,
				or keep a copy somewhere safer, save it to a file above.
			</footer>
		</div>
	</div>
{/if}

<style>
	/* ==========================================================================
	   Your designs. Ported from Designs.css.

	   Owns .dz-* only, and reuses .btn and the tokens from app.css. Structure
	   and motion mirror the shortcuts and examples dialogs, because it is the
	   same kind of object: something consulted between actions, not during
	   one.
	   ========================================================================== */

	.dz-root {
		position: fixed;
		inset: 0;
		z-index: 500;
		display: grid;
		place-items: center;
		padding: var(--sp-5);
	}

	.dz-scrim {
		position: absolute;
		inset: 0;
		background: var(--scrim);
		cursor: default;
		animation: dz-scrim-in var(--dur-slow) var(--ease-out);
	}

	.dz-card {
		position: relative;
		display: flex;
		flex-direction: column;
		width: min(520px, 100%);
		max-height: min(640px, 100%);
		min-height: 0;
		background: var(--surface);
		border: var(--bw) solid var(--border-strong);
		border-radius: var(--r-lg);
		box-shadow: var(--shadow-lg);
		outline: none;
		animation: dz-card-in var(--dur-slow) var(--ease-out);
	}

	.dz-root.is-closing .dz-card {
		animation: dz-card-out var(--dur-base) var(--ease) forwards;
	}

	.dz-root.is-closing .dz-scrim {
		animation: dz-scrim-out var(--dur-base) var(--ease) forwards;
	}

	@keyframes dz-card-in {
		from {
			opacity: 0;
			transform: translateY(6px);
		}
		to {
			opacity: 1;
			transform: translateY(0);
		}
	}

	@keyframes dz-card-out {
		from {
			opacity: 1;
		}
		to {
			opacity: 0;
			transform: translateY(4px);
		}
	}

	@keyframes dz-scrim-in {
		from {
			opacity: 0;
		}
		to {
			opacity: 1;
		}
	}

	@keyframes dz-scrim-out {
		from {
			opacity: 1;
		}
		to {
			opacity: 0;
		}
	}

	.dz-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--sp-3);
		padding: var(--sp-4) var(--sp-5) var(--sp-3);
		border-bottom: var(--bw) solid var(--border);
	}

	.dz-title {
		margin: 0;
		font-size: var(--fs-base);
		font-weight: var(--fw-med);
		letter-spacing: var(--tr-base);
		color: var(--text);
	}

	/* Save first, list second: the reader opened this to keep what is on the
	   canvas at least as often as to fetch something back. */
	.dz-save {
		display: flex;
		gap: var(--sp-2);
		padding: var(--sp-4) var(--sp-5) var(--sp-3);
	}

	.dz-name {
		flex: 1 1 auto;
		min-width: 0;
		height: 32px;
		padding: 0 var(--sp-3);
		border: var(--bw) solid var(--border-strong);
		border-radius: var(--r-btn);
		background: var(--surface-2);
		color: var(--text);
		font: inherit;
		font-size: var(--fs-base);
	}

	.dz-name::placeholder {
		color: var(--text-faint);
	}

	.dz-name:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: -1px;
		border-color: var(--accent);
	}

	/* File import/export: a second, quieter row of actions, ghost buttons so
	   they do not compete with the primary Save above them. */
	.dz-file-actions {
		display: flex;
		gap: var(--sp-2);
		padding: 0 var(--sp-5) var(--sp-3);
	}

	.dz-error {
		margin: 0;
		padding: 0 var(--sp-5) var(--sp-2);
		font-size: var(--fs-sm);
		color: var(--danger);
	}

	.dz-body {
		min-height: 0;
		overflow-y: auto;
		padding: 0 var(--sp-5) var(--sp-3);
		scrollbar-width: thin;
		scrollbar-color: var(--line) transparent;
	}

	.dz-empty {
		margin: 0;
		padding: var(--sp-4) 0 var(--sp-5);
		font-size: var(--fs-sm);
		line-height: var(--lh-sm);
		color: var(--text-dim);
	}

	.dz-list {
		display: flex;
		flex-direction: column;
		gap: 2px;
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.dz-item {
		display: flex;
		align-items: center;
		gap: var(--sp-2);
		border-radius: var(--r-btn);
	}

	.dz-item:hover {
		background: var(--surface-2);
	}

	.dz-open {
		display: flex;
		flex: 1 1 auto;
		flex-direction: column;
		gap: 1px;
		min-width: 0;
		padding: var(--sp-2);
		border: none;
		border-radius: var(--r-btn);
		background: none;
		font: inherit;
		text-align: left;
		cursor: pointer;
	}

	.dz-open:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: -2px;
	}

	.dz-item-name {
		overflow: hidden;
		color: var(--text);
		font-size: var(--fs-base);
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.dz-item-meta {
		color: var(--text-dim);
		font-size: var(--fs-sm);
	}

	.dz-actions {
		display: flex;
		flex: none;
		gap: 2px;
		padding-right: var(--sp-2);
		opacity: 0;
		transition: opacity var(--dur-fast) var(--ease);
	}

	.dz-item:hover .dz-actions,
	.dz-item:focus-within .dz-actions {
		opacity: 1;
	}

	.dz-delete:hover {
		color: var(--danger);
	}

	.dz-rename {
		margin: var(--sp-1) 0 var(--sp-1) var(--sp-2);
	}

	.dz-foot {
		padding: var(--sp-3) var(--sp-5) var(--sp-4);
		border-top: var(--bw) solid var(--border);
		font-size: var(--fs-sm);
		line-height: var(--lh-sm);
		color: var(--text-dim);
	}

	@media (prefers-reduced-motion: reduce) {
		.dz-card,
		.dz-scrim,
		.dz-root.is-closing .dz-card,
		.dz-root.is-closing .dz-scrim {
			animation-duration: 0.01ms;
		}

		.dz-actions {
			transition: none;
		}
	}
</style>
