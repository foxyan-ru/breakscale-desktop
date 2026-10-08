<script lang="ts">
  /* ==========================================================================
     Note format toolbar -- S/M/L size, bold/italic/underline, typeface and
     text-colour tone, for the one selected note.

     Field-for-field port of the web app's format bar: `formatNote`/
     `onSetNoteSize`/`onSetNoteStyle` and their JSX (`src/components/Canvas.tsx`
     ~5193 "the one selected note, or null" and ~6113-6207 the `.cv-format`
     markup), driven by `handleSetNoteSize`/`handleSetNoteStyle` in
     `src/App.tsx` (~1382-1428). This is PRE-EXISTING web functionality --
     present in the local pinned reference copy (`../src`), predating the
     upstream WYSIWYG delta -- that the desktop port never wired a control
     for; Canvas.svelte's own "Known limitations" comment called it out by
     name ("the original's bold/italic/underline/font/tone toolbar... not
     wired to any UI control in this pass"). docs/PORTING_GAP.md groups it
     with upstream `3ce685bd`/#76 because that PR is explicit that this bar
     "must be built on top of" the new WYSIWYG editor rather than the old
     plain-textarea one, not because the PR itself added the bar.

     Bold/italic/underline are stored ABSENT when off, never `false`
     (`Note`'s own doc comments): `onSetStyle` mirrors web's `toggle` shape
     rather than a plain boolean so the caller (Canvas.svelte) can delete
     the key the same way `handleSetNoteStyle` does, instead of writing a
     redundant `false`.

     Fixed to the bottom of the canvas rather than anchored beside the note
     it edits -- web's own comment explains why: a toolbar this wide clips
     against the viewport edge next to a default-width note, and one
     anchored to the note slides under the reader's hand on every pan. Every
     editor with a rich-text object puts its format bar here for the same
     reason.

     Rendered by the caller as a DOM SIBLING of `.cv-surface` (Canvas.svelte's
     own floating-chrome rule, see that file's header), so a press here can
     never reach the canvas's pointer router.
     ========================================================================== */

  import { ANNOTATION_FONTS } from '$lib/domain/annotations';
  import type { AnnotationFont, Note } from '$lib/domain/annotations';

  /** Mirrors web's `onSetNoteStyle` change shape (`App.tsx` ~1393-1428). */
  export interface NoteStyleChange {
    font?: AnnotationFont;
    /** `null` means "follow the text colour" (no tone at all). */
    tone?: number | null;
    bold?: 'toggle';
    italic?: 'toggle';
    underline?: 'toggle';
  }

  interface Props {
    note: Note;
    /** `SECTION_TONE_COUNT` -- the same 13-shade palette sections use. */
    toneCount: number;
    onSetSize: (size: Note['size']) => void;
    onSetStyle: (change: NoteStyleChange) => void;
  }

  let { note, toneCount, onSetSize, onSetStyle }: Props = $props();

  const SIZES: readonly Note['size'][] = ['sm', 'md', 'lg'];
  const SIZE_LABEL: Record<Note['size'], string> = { sm: 'S', md: 'M', lg: 'L' };
  /** Names for the face buttons, which all read "Aa" set in their own face. */
  const FONT_LABEL: Record<AnnotationFont, string> = {
    sans: 'Interface',
    hand: 'Handwritten',
    serif: 'Serif',
    mono: 'Monospace',
  };

  const tones = $derived(Array.from({ length: toneCount }, (_, i) => i));
</script>

<div class="cv-format" data-chrome="format">
  <div class="cv-format-group" role="group" aria-label="Text size">
    {#each SIZES as size (size)}
      <button
        type="button"
        class="btn btn-ghost cv-format-btn"
        class:is-active={note.size === size}
        aria-pressed={note.size === size}
        title={`Size ${size}`}
        onclick={() => onSetSize(size)}
      >
        {SIZE_LABEL[size]}
      </button>
    {/each}
    <button
      type="button"
      class="btn btn-ghost cv-format-btn cv-format-bold"
      class:is-active={note.bold === true}
      aria-pressed={note.bold === true}
      title="Bold"
      onclick={() => onSetStyle({ bold: 'toggle' })}
    >
      B
    </button>
    <button
      type="button"
      class="btn btn-ghost cv-format-btn cv-format-italic"
      class:is-active={note.italic === true}
      aria-pressed={note.italic === true}
      title="Italic"
      onclick={() => onSetStyle({ italic: 'toggle' })}
    >
      I
    </button>
    <button
      type="button"
      class="btn btn-ghost cv-format-btn cv-format-underline"
      class:is-active={note.underline === true}
      aria-pressed={note.underline === true}
      title="Underline"
      onclick={() => onSetStyle({ underline: 'toggle' })}
    >
      U
    </button>
  </div>

  <div class="cv-format-group" role="group" aria-label="Typeface">
    {#each ANNOTATION_FONTS as f (f)}
      <button
        type="button"
        class="btn btn-ghost cv-format-btn"
        data-font-name={f}
        class:is-active={(note.font ?? 'sans') === f}
        aria-pressed={(note.font ?? 'sans') === f}
        title={FONT_LABEL[f]}
        onclick={() => onSetStyle({ font: f })}
      >
        Aa
      </button>
    {/each}
  </div>

  <div class="cv-format-group" role="group" aria-label="Text colour">
    {#each tones as i (i)}
      <button
        type="button"
        class="cv-format-tone"
        class:is-active={note.tone === i}
        data-tone={i}
        style={`--ann-tone-fill: var(--ann-${i}-fill); border-color: var(--ann-${i}-line);`}
        aria-pressed={note.tone === i}
        aria-label={`Colour ${i + 1}`}
        onclick={() => onSetStyle({ tone: i })}
      ></button>
    {/each}
    <button
      type="button"
      class="cv-format-tone cv-format-tone-none"
      class:is-active={note.tone === undefined}
      aria-pressed={note.tone === undefined}
      aria-label="Default colour"
      title="Follow the text colour"
      onclick={() => onSetStyle({ tone: null })}
    ></button>
  </div>
</div>

<style>
  /* Ported from the web app's Canvas.css `.cv-format` rules. Scoped to this
     component rather than added to Canvas.css, which that file's own header
     reserves for Canvas.svelte's own chrome classes. */
  .cv-format {
    position: absolute;
    left: 50%;
    transform: translateX(-50%);
    bottom: calc(var(--sp-3) + 32px + var(--sp-3));
    z-index: 9;
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    padding: var(--sp-1);
    background: var(--surface);
    border: var(--bw) solid var(--border);
    border-radius: var(--r-md);
    box-shadow: var(--shadow-md);
  }

  .cv-format-group {
    display: flex;
    align-items: center;
    gap: 2px;
  }

  .cv-format-group + .cv-format-group {
    padding-left: var(--sp-2);
    border-left: var(--bw) solid var(--border);
  }

  .cv-format-btn {
    min-width: 30px;
    height: 30px;
    padding: 0 var(--sp-1);
  }

  .cv-format-btn.is-active {
    background: var(--accent-soft);
    border-color: var(--accent);
    color: var(--accent-ink);
  }

  .cv-format-bold {
    font-weight: 700;
  }

  .cv-format-italic {
    font-style: italic;
  }

  .cv-format-underline {
    text-decoration: underline;
  }

  /* Each face button is SET in the face it selects, so the row is a
     specimen rather than four identical letters that have to be learned. */
  .cv-format-btn[data-font-name='hand'] {
    font-family: var(--hand);
  }

  .cv-format-btn[data-font-name='serif'] {
    font-family: var(--serif);
  }

  .cv-format-btn[data-font-name='mono'] {
    font-family: var(--mono);
  }

  .cv-format-tone {
    width: 20px;
    height: 20px;
    padding: 0;
    border: var(--bw) solid var(--border-strong);
    border-radius: var(--r-sm);
    background: var(--ann-tone-fill, var(--surface-2));
    cursor: pointer;
  }

  .cv-format-tone:hover {
    border-color: var(--text-dim);
  }

  /* Width only, not colour: a hue swatch's border-color comes from its own
     inline style (always wins over this class rule), matching the web
     original's equal-specificity tiebreak between `.is-active` and
     `[data-tone='N']`. The "no colour" swatch below has no inline
     border-color, so it IS this rule's `border-color: var(--text)`. */
  .cv-format-tone.is-active {
    border-color: var(--text);
    border-width: 2px;
  }

  .cv-format-tone:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  /* "No colour": a slash, because an empty swatch reads as white, which is
     a colour and not the absence of one. */
  .cv-format-tone-none {
    background:
      linear-gradient(
        to bottom right,
        transparent calc(50% - 1px),
        var(--text-faint) calc(50% - 1px),
        var(--text-faint) calc(50% + 1px),
        transparent calc(50% + 1px)
      ),
      var(--surface-2);
  }
</style>
