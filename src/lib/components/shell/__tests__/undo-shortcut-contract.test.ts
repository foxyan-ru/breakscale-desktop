import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The undo/redo feature spans seven files this suite can only read: the
 * history module, `+page.svelte` (keys, buttons, receipt), `shell.css`
 * (the two styles those buttons/toast need), Canvas.svelte (gesture
 * brackets + every discrete canvas edit), Inspector.svelte (knob
 * streams), Designs.svelte (design/file loads) and `Shortcuts.svelte`
 * (the dialog that advertises the chords). jsdom can click what renders,
 * but it cannot prove that the shortcuts DIALOG still matches the chords
 * the page actually handles -- which is the drift these pin, following
 * `bar-clear.test.ts`'s source-level precedent for anything layout- or
 * table-shaped -- nor that every topology write path still carries its
 * history label, which is the second `it` below.
 */
describe('undo/redo wiring contract', () => {
  const read = (path: string): string => readFileSync(resolve(process.cwd(), path), 'utf8');
  const pageSource = read('src/routes/+page.svelte');
  const shortcutsSource = read('src/lib/components/shell/Shortcuts.svelte');
  const shellCss = read('src/lib/components/shell/shell.css');
  const historySource = read('src/lib/state/history.svelte.ts');
  const canvasSource = read('src/lib/components/canvas/Canvas.svelte');
  const inspectorSource = read('src/lib/components/inspector/Inspector.svelte');
  const designsSource = read('src/lib/components/shell/Designs.svelte');

  it('documents exactly the chords the page handles', () => {
    // The dialog has advertised these rows since it was ported; until this
    // task they bound nothing.
    expect(shortcutsSource).toContain("{ keys: ['Ctrl+Z'], does: 'Undo' }");
    expect(shortcutsSource).toContain("{ keys: ['Ctrl+Shift+Z', 'Ctrl+Y'], does: 'Redo' }");

    // And the page now handles them: e.code (not e.key) so the binding
    // survives a non-QWERTY layout, Alt never triggers, Shift flips
    // Ctrl+Z over to redo, Ctrl+Y is a straight redo.
    expect(pageSource).toContain("e.code === 'KeyZ'");
    expect(pageSource).toContain("e.code === 'KeyY'");
    expect(pageSource).toContain('if (e.shiftKey) handleRedo();');
    expect(pageSource).toContain('!e.altKey');
  });

  it('guards the chords behind the text-field check, before anything else', () => {
    const guard = pageSource.indexOf('if (isTypingTarget(e.target)) return;');
    const undoChord = pageSource.indexOf("e.code === 'KeyZ'");
    expect(guard).toBeGreaterThan(-1);
    expect(undoChord).toBeGreaterThan(guard);
    // Handled chords return: one Ctrl+Z must not also toggle a panel.
    expect(pageSource).toContain('e.preventDefault();');
  });

  it('renders both controls as real buttons whose disabled state is derived', () => {
    expect(pageSource).toContain('<div class="app-history">');
    expect(pageSource).toContain('aria-label="Undo"');
    expect(pageSource).toContain('aria-label="Redo"');
    expect(pageSource).toContain('disabled={!sessionHistory.canUndo}');
    expect(pageSource).toContain('disabled={!sessionHistory.canRedo}');
    // No cached boolean anywhere near them: Excalidraw's stale-button bug
    // was exactly a canUndo remembered across renders.
    expect(pageSource).not.toContain('let canUndo');
    expect(pageSource).not.toContain('let canRedo');
  });

  it('raises a role=status receipt keyed by the receipt id', () => {
    expect(pageSource).toContain('<div class="app-toast" role="status">{receipt.text}</div>');
    expect(pageSource).toContain('{#key receipt.id}');
    expect(pageSource).toContain('{#if sessionHistory.receipt}');
    // Both styles already exist in the shell stylesheet this app shares.
    expect(shellCss).toContain('.app-history');
    expect(shellCss).toContain('.app-toast');
    expect(shellCss).toContain('app-toast-in');
  });

  it('wires commits, touches and entry application in the page', () => {
    expect(pageSource).toContain('sessionHistory.undo(currentSnapshot())');
    expect(pageSource).toContain('sessionHistory.redo(currentSnapshot())');
    expect(pageSource).toContain('setSelection(entry.selectedIds)');
    // Discrete edits name themselves; streamed ones coalesce.
    expect(pageSource).toContain("sessionHistory.commit('add'");
    expect(pageSource).toContain("sessionHistory.commit('example load'");
    expect(pageSource).toContain("sessionHistory.commit('challenge start'");
    expect(pageSource).toContain("sessionHistory.touch('setting change'");
  });

  it('keeps the history module the documented web-port it is', () => {
    expect(historySource).toContain('HISTORY_LIMIT = 50');
    expect(historySource).toContain('SETTLE_MS = 500');
    expect(historySource).toContain('RECEIPT_MS = 2200');
    expect(historySource).toContain('export const sessionHistory = createSessionHistory()');
  });

  it('brackets every draggable canvas gesture, and closes it on every exit', () => {
    // promote(): node move, annotation move, section resize, note resize.
    expect(canvasSource.match(/sessionHistory\.beginGesture\(/g) ?? []).toHaveLength(4);
    // pointerup, pointercancel, Escape (cancelGesture) and the pinch that
    // takes the surface over mid-drag.
    expect(canvasSource.match(/closeUndoGesture\(\);/g) ?? []).toHaveLength(4);
    // Panning, marqueeing, linking and drawing are NOT gestures in the web
    // app either, so they must not open brackets.
    expect(canvasSource).not.toContain("beginGesture('pan'");
    expect(canvasSource).not.toContain("beginGesture('marquee'");
    expect(canvasSource).not.toContain("beginGesture('link'");
  });

  it('pins every topology-edit label at the file that owns the write', () => {
    // The EXACT label is what the undo receipt prints and what entries are
    // told apart by, so each call site is counted here: a second commit
    // sneaking in (double entries) or one disappearing (an un-undoable
    // write) fails this test. Counts are of `sessionHistory.`-prefixed
    // calls only, so prose in the WHY comments does not participate.
    const count = (src: string, re: RegExp): number => src.match(re)?.length ?? 0;

    // Canvas.svelte -- web label and App.tsx line ref sit beside each call.
    expect(count(canvasSource, /sessionHistory\.commit\('delete'/g)).toBe(4); // blanked note, edge X, Delete key, cut
    expect(count(canvasSource, /sessionHistory\.commit\('connection'/g)).toBe(3); // drag-release link, click-through, keyboard Enter
    expect(count(canvasSource, /sessionHistory\.commit\('add note'/g)).toBe(1); // tool click, palette drop (shared createNote)
    expect(count(canvasSource, /sessionHistory\.commit\('add section'/g)).toBe(1); // tool click, section draw, palette drop
    expect(count(canvasSource, /sessionHistory\.commit\('note edit'/g)).toBe(1);
    expect(count(canvasSource, /sessionHistory\.commit\('label edit'/g)).toBe(1);
    expect(count(canvasSource, /sessionHistory\.commit\('section shade'/g)).toBe(1);
    expect(count(canvasSource, /sessionHistory\.commit\('paste'/g)).toBe(1);
    expect(count(canvasSource, /sessionHistory\.commit\('add'/g)).toBe(1); // palette drop of a node
    expect(count(canvasSource, /sessionHistory\.touch\('rename'/g)).toBe(1);
    expect(count(canvasSource, /sessionHistory\.touch\('move'/g)).toBe(1); // arrow-key nudge
    // Handle moves/resizes are bracketed, not touched -- the web's own
    // guarded touches at :1156/:1234-1278 no-op inside a gesture too.
    expect(count(canvasSource, /sessionHistory\.touch\('resize'/g)).toBe(0);

    // Inspector.svelte -- every knob rides one coalescing label.
    expect(count(inspectorSource, /sessionHistory\.touch\('setting change'/g)).toBe(2);

    // Designs.svelte -- the two load paths, labelled like App.tsx's
    // replaceDesign calls (:1912 / :2131).
    expect(count(designsSource, /sessionHistory\.commit\('open design'/g)).toBe(1);
    expect(count(designsSource, /sessionHistory\.commit\('file import'/g)).toBe(1);

    // +page.svelte -- the palette add, both loads, and both config paths.
    expect(count(pageSource, /sessionHistory\.commit\('add'/g)).toBe(1);
    expect(count(pageSource, /sessionHistory\.commit\('example load'/g)).toBe(1);
    expect(count(pageSource, /sessionHistory\.commit\('challenge start'/g)).toBe(1);
    expect(count(pageSource, /sessionHistory\.touch\('setting change'/g)).toBe(2);

    // 'duplicate'/'note size'/'note style' are web labels with NO desktop
    // write path (those features do not exist here) -- their absence is
    // the honest divergence, so pin it rather than inventing sites.
    expect(canvasSource).not.toContain("commit('duplicate'");
    expect(canvasSource).not.toContain("commit('note size'");
    expect(canvasSource).not.toContain("commit('note style'");
  });
});
