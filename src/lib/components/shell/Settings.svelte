<script lang="ts">
	/* ==========================================================================
	   Settings.

	   Svelte port of src/components/Settings.tsx. Same dialog chrome as
	   Shortcuts.svelte/Designs.svelte in this directory (mounted/closing
	   presence, scrim, focus-trapped card) -- see Shortcuts.svelte for the
	   fuller rationale on why a dialog rather than a permanent rail.

	   UNLIKE THE REACT VERSION, this component does not take onExport/
	   onImport/onBackup/onRestore callback props. The React app owned that
	   logic in App.tsx and handed Settings six callbacks because designs and
	   backups both lived in localStorage App.tsx already had open; here,
	   "your design" file actions (save/open/copy link/export image) live with
	   the Designs dialog, which already owns the design-file API
	   (`$lib/api/designs.ts` + `$lib/api/dialog.ts`) -- see Designs.svelte's
	   own header for why that dialog calls its APIs directly rather than via
	   props. Settings therefore keeps only what is genuinely its own: theme,
	   vendor naming, canvas preferences, and the whole-app backup/restore
	   pair (`$lib/api/backup.ts`), wired directly the same way.

	   Preferences write straight to `settingsStore` via `setSetting` -- see
	   that store's own TODO about a future settings_load/settings_save Tauri
	   command; until then this is in-memory only, matching its doc comment.
	   ========================================================================== */

	import { settingsStore, setSetting, THEME_CHOICES } from '$lib/state/settings.svelte';
	import type { ThemeChoice, Settings } from '$lib/state/settings.svelte';
	import { themeState } from '$lib/state/theme.svelte';
	import type { VendorId } from '$lib/domain';
	import { BACKUP_EXT } from '$lib/domain';
	import { backupWrite, backupRestoreFromPath } from '$lib/api/backup';
	import { pickOpenPath, pickSavePath } from '$lib/api/dialog';
	import { isAppError } from '$lib/api';
	import { pushError } from '$lib/state/ui.svelte';

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

	/* Focus lands on the card at open and returns to the opener at close,
	   the same contract every dialog in this directory keeps. */
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

	/* Reset the backup panel's own transient state each time the dialog
	   opens, so a stale error/notice from a previous visit does not linger. */
	$effect(() => {
		if (!open) return;
		backupError = null;
		backupNotice = null;
	});

	function onKeyDown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			e.preventDefault();
			e.stopPropagation();
			onClose();
		}
	}

	/* ---------------------------------------------------------------- *
	 * Appearance: theme.
	 * ---------------------------------------------------------------- */

	const THEME_LABELS: Record<ThemeChoice, string> = {
		light: 'Light',
		dark: 'Dark',
		system: 'System',
	};

	/* ---------------------------------------------------------------- *
	 * Naming: vendor.
	 *
	 * `$lib/domain/vendors.ts` types `VendorId` and the full `Vendor` spec
	 * shape (sizes, pricing, citations) but, unlike the web app's
	 * `src/content/vendors/index.ts`, does not export a small display-name
	 * list -- that module ports the cost-model *data*, which is out of
	 * scope for this pass (MIGRATION_PLAN.md Â§4 lists `vendors::{types,
	 * cost, derive}` + `data/vendors/*.json` as their own port). The four
	 * labels below are copied from the web app's own `VENDORS` array
	 * (`src/content/vendors/index.ts` lines 22-26) so the wording matches.
	 * ---------------------------------------------------------------- */

	const VENDORS: { id: VendorId; label: string }[] = [
		{ id: 'generic', label: 'Generic' },
		{ id: 'aws', label: 'AWS' },
		{ id: 'gcp', label: 'Google' },
		{ id: 'azure', label: 'Azure' },
	];

	/* ---------------------------------------------------------------- *
	 * Canvas preference toggles.
	 * ---------------------------------------------------------------- */

	const TOGGLES: {
		key: Exclude<keyof Settings, 'theme' | 'vendor'>;
		label: string;
		hint: string;
	}[] = [
		{
			key: 'tooltips',
			label: 'Explain metric names',
			hint: 'Underline terms like p99 and show what they mean on hover.',
		},
		{
			key: 'sparklines',
			label: 'Trend lines on components',
			hint: 'Draw the recent history of each component on its box.',
		},
		{
			key: 'snapToGrid',
			label: 'Snap to the grid',
			hint: 'Keep components aligned while dragging. G toggles it, and holding Ctrl bypasses it for one drag.',
		},
		{
			key: 'minimap',
			label: 'Minimap',
			hint: 'A small map of the whole diagram, for finding your way around a big one.',
		},
	];

	/* ---------------------------------------------------------------- *
	 * Backup / restore: backupWrite / backupRestoreFromPath
	 * (`$lib/api/backup.ts`) + pickSavePath / pickOpenPath
	 * (`$lib/api/dialog.ts`), the same two-step "resolve a path, then call
	 * the command that takes it" pattern Designs.svelte uses for its own
	 * file export/import.
	 * ---------------------------------------------------------------- */

	const BACKUP_FILTERS = [{ name: 'Breakscale backup', extensions: ['json'] }];

	let backupBusy = $state(false);
	let restoreBusy = $state(false);
	/** Shown inline, next to the action that failed -- never an unhandled rejection. */
	let backupError = $state<string | null>(null);
	let backupNotice = $state<string | null>(null);

	function describeError(action: string, e: unknown): string {
		const message = isAppError(e) ? e.message : e instanceof Error ? e.message : String(e);
		return `${action} failed: ${message}`;
	}

	function suggestedBackupName(): string {
		const now = new Date();
		const y = now.getFullYear();
		const m = String(now.getMonth() + 1).padStart(2, '0');
		const d = String(now.getDate()).padStart(2, '0');
		return `breakscale-${y}-${m}-${d}${BACKUP_EXT}`;
	}

	async function doBackup(): Promise<void> {
		backupBusy = true;
		backupError = null;
		backupNotice = null;
		try {
			const path = await pickSavePath(suggestedBackupName(), BACKUP_FILTERS);
			if (!path) return;
			await backupWrite(path);
			backupNotice = 'Backup saved. This one file holds every saved design, your settings and the canvas you have open.';
		} catch (e) {
			const message = describeError('Creating the backup', e);
			backupError = message;
			pushError(message);
		} finally {
			backupBusy = false;
		}
	}

	async function doRestore(): Promise<void> {
		restoreBusy = true;
		backupError = null;
		backupNotice = null;
		try {
			const path = await pickOpenPath(BACKUP_FILTERS);
			if (!path) return;
			// backupRestoreFromPath resolves even when the file is rejected -- it
			// is a BackupResult discriminated union, not always a success. Both
			// branches are handled explicitly; only a genuine IPC failure (a
			// rejected promise) falls to the catch below.
			const result = await backupRestoreFromPath(path);
			if (result.ok) {
				backupNotice = `Restored: ${result.restored.join(', ')}. Restart Breakscale to see everything.`;
			} else {
				backupError = result.error;
			}
		} catch (e) {
			const message = describeError('Restoring the backup', e);
			backupError = message;
			pushError(message);
		} finally {
			restoreBusy = false;
		}
	}
</script>

{#if mounted}
	<div class={`se-root${closing ? ' is-closing' : ''}`} inert={closing || undefined}>
		<div class="se-scrim" onclick={onClose} aria-hidden="true"></div>
		<div
			bind:this={cardEl}
			class="se-card"
			role="dialog"
			aria-modal="true"
			aria-labelledby="se-title"
			tabindex="-1"
			onkeydown={onKeyDown}
			onanimationend={handleAnimationEnd}
		>
			<header class="se-head">
				<h2 id="se-title" class="se-title">Settings</h2>
				<button
					type="button"
					class="btn btn-ghost btn-sm btn-icon"
					onclick={onClose}
					aria-label="Close settings"
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

			<div class="se-body scroll">
				<section class="se-group">
					<h3 class="se-group-title">Appearance</h3>
					<div class="se-choice" role="group" aria-labelledby="se-theme-label">
						<span id="se-theme-label" class="row-k">Theme</span>
						<div class="se-seg">
							{#each THEME_CHOICES as t (t)}
								<button
									type="button"
									class="btn btn-sm"
									aria-pressed={settingsStore.theme === t}
									onclick={() => setSetting('theme', t)}
								>
									{THEME_LABELS[t]}
								</button>
							{/each}
						</div>
					</div>
					<p class="prose se-hint">
						{settingsStore.theme === 'system'
							? `Following your device, which is currently ${themeState.resolvedTheme}.`
							: 'Kept across visits, whatever your device is set to.'}
					</p>
				</section>

				<section class="se-group">
					<h3 class="se-group-title">Naming</h3>
					<div class="se-choice" role="group" aria-labelledby="se-vendor-label">
						<span id="se-vendor-label" class="row-k">Component names</span>
						<div class="se-seg">
							{#each VENDORS as v (v.id)}
								<button
									type="button"
									class="btn btn-sm"
									aria-pressed={settingsStore.vendor === v.id}
									onclick={() => setSetting('vendor', v.id)}
								>
									{v.label}
								</button>
							{/each}
						</div>
					</div>
					<p class="prose se-hint">
						{settingsStore.vendor === 'generic'
							? 'Components keep their plain names. Learn the idea first; the product names are easier afterwards.'
							: 'Components are named after this vendorâ€™s products. The published specs are cited; how they map to capacity is our own estimate.'}
					</p>
				</section>

				<section class="se-group">
					<h3 class="se-group-title">Canvas</h3>
					{#each TOGGLES as row (row.key)}
						<div class="field">
							<label class="row-k" for={`se-toggle-${row.key}`}>{row.label}</label>
							<input
								id={`se-toggle-${row.key}`}
								type="checkbox"
								checked={settingsStore[row.key]}
								onchange={(e) =>
									setSetting(row.key, (e.currentTarget as HTMLInputElement).checked)}
							/>
						</div>
						<p class="prose se-hint">{row.hint}</p>
					{/each}
				</section>

				<section class="se-group">
					<h3 class="se-group-title">This machine</h3>
					<p class="prose se-hint">
						One file with every saved design, your settings and the canvas you have open.
						This is how you move to another machine.
					</p>
					<div class="se-actions">
						<button
							type="button"
							class="btn btn-secondary btn-sm"
							onclick={doBackup}
							disabled={backupBusy}
						>
							{backupBusy ? 'Savingâ€¦' : 'Download everythingâ€¦'}
						</button>
						<button
							type="button"
							class="btn btn-secondary btn-sm"
							onclick={doRestore}
							disabled={restoreBusy}
						>
							{restoreBusy ? 'Restoringâ€¦' : 'Restore from a fileâ€¦'}
						</button>
					</div>
					{#if backupError}
						<p class="se-error" role="alert">{backupError}</p>
					{:else if backupNotice}
						<p class="se-notice" role="status">{backupNotice}</p>
					{/if}
				</section>
			</div>
		</div>
	</div>
{/if}

<style>
	/* ==========================================================================
	   Settings dialog. Chrome (.se-root/.se-scrim/.se-card/.se-head/.se-title)
	   matches Shortcuts.svelte and Designs.svelte in this directory, kept
	   identical on purpose so every "reference or setup" dialog in the shell
	   opens and closes the same way. Everything below the header reuses
	   app.css's .field/.label/.prose/.btn primitives rather than redeclaring
	   row/label/control styling; only what those primitives do not cover
	   (the segmented theme/vendor picker, the group spacing, the inline
	   backup status lines) is defined here.
	   ========================================================================== */

	.se-root {
		position: fixed;
		inset: 0;
		z-index: 500;
		display: grid;
		place-items: center;
		padding: var(--sp-5);
	}

	.se-scrim {
		position: absolute;
		inset: 0;
		background: var(--scrim);
		cursor: default;
		animation: se-scrim-in var(--dur-slow) var(--ease-out);
	}

	.se-card {
		position: relative;
		display: flex;
		flex-direction: column;
		width: min(460px, 100%);
		max-height: min(640px, 100%);
		min-height: 0;
		background: var(--surface);
		border: var(--bw) solid var(--border-strong);
		border-radius: var(--r-lg);
		box-shadow: var(--shadow-lg);
		outline: none;
		animation: se-card-in var(--dur-slow) var(--ease-out);
	}

	.se-root.is-closing .se-card {
		animation: se-card-out var(--dur-base) var(--ease) forwards;
	}

	.se-root.is-closing .se-scrim {
		animation: se-scrim-out var(--dur-base) var(--ease) forwards;
	}

	@keyframes se-card-in {
		from {
			opacity: 0;
			transform: translateY(6px);
		}
		to {
			opacity: 1;
			transform: translateY(0);
		}
	}

	@keyframes se-card-out {
		from {
			opacity: 1;
			transform: translateY(0);
		}
		to {
			opacity: 0;
			transform: translateY(4px);
		}
	}

	@keyframes se-scrim-in {
		from {
			opacity: 0;
		}
		to {
			opacity: 1;
		}
	}

	@keyframes se-scrim-out {
		from {
			opacity: 1;
		}
		to {
			opacity: 0;
		}
	}

	.se-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--sp-3);
		padding: var(--sp-4) var(--sp-5) var(--sp-3);
		border-bottom: var(--bw) solid var(--border);
	}

	.se-title {
		margin: 0;
		font-size: var(--fs-base);
		font-weight: var(--fw-med);
		letter-spacing: var(--tr-base);
		color: var(--text);
	}

	.se-body {
		min-height: 0;
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		gap: var(--sp-7);
		padding: var(--sp-4) var(--sp-5) var(--sp-5);
	}

	.se-group {
		display: flex;
		flex-direction: column;
		gap: var(--sp-2);
	}

	.se-group-title {
		margin: 0;
		font-size: var(--fs-label);
		font-weight: var(--fw-num);
		letter-spacing: var(--tr-label);
		text-transform: uppercase;
		color: var(--text-faint);
	}

	/* A field row whose control is a button group rather than an input/select,
	   so it cannot reuse .field's input/select sizing -- everything else about
	   it (label left, control right, same min-height) matches .field. */
	.se-choice {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--sp-3);
		min-height: 36px;
	}

	.se-seg {
		display: inline-flex;
		gap: var(--sp-1);
		flex-wrap: wrap;
		justify-content: flex-end;
	}

	/* .row-k has no flex-basis of its own (unlike .field's own .label child,
	   which app.css gives min-width: 0); without it a long row label would
	   push the segmented control past the card edge instead of wrapping. */
	.se-choice > .row-k,
	.field > .row-k {
		min-width: 0;
	}

	/* Hint prose sits directly under the row it explains, so it needs less
	   top gap than the .se-group flex gap gives every other child. */
	.se-hint {
		margin-top: calc(var(--sp-2) * -1);
	}

	/* Fix up the generic `.field > input` rule (app.css), which sizes every
	   direct-child input to a fixed 96px assuming a number field -- correct
	   there, wrong for the 18px checkbox a toggle row uses here. */
	.field > input[type='checkbox'] {
		flex: 0 0 auto;
		width: 18px;
	}

	.se-actions {
		display: flex;
		gap: var(--sp-2);
		flex-wrap: wrap;
	}

	.se-error,
	.se-notice {
		margin: 0;
		font-size: var(--fs-sm);
		line-height: var(--lh-sm);
	}

	.se-error {
		color: var(--danger);
	}

	.se-notice {
		color: var(--ok);
	}

	@media (prefers-reduced-motion: reduce) {
		.se-card,
		.se-scrim,
		.se-root.is-closing .se-card,
		.se-root.is-closing .se-scrim {
			animation-duration: 0.01ms;
		}
	}
</style>
