import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * `--bar-clear` (shell.css) is how far below the floating top bar the
 * slots, the metrics strip and the inspector's close button start. It was
 * a hand-maintained constant, and it stopped fitting the moment the bar's
 * contents changed: the traffic island that replaced the two text buttons
 * is far taller, and on a narrow window the bar wraps past 200px. An
 * under-measured clearance is not a cosmetic miss -- the bar sits at
 * z-index 40 above `.app-slot` (z-index 10), so the inspector's close
 * button lands underneath it and the clicks go to the bar instead.
 *
 * The web app solved this by MEASURING the bar (App.tsx:668-718) and
 * writing the value inline; these tests pin that same mechanism in
 * `+page.svelte`, plus the stylesheet constant that still stands in for
 * the first paint (and for any environment without a ResizeObserver).
 *
 * Source-level, like `sysdesign/scroll-contract.test.ts`: jsdom has no
 * layout, so there is no bar to measure -- what matters is that the page
 * measures it in a browser, and the source says so.
 */
describe('bar clearance', () => {
  const pageSource = readFileSync(resolve(process.cwd(), 'src/routes/+page.svelte'), 'utf8');
  const shellCss = readFileSync(
    resolve(process.cwd(), 'src/lib/components/shell/shell.css'),
    'utf8',
  );

  it('derives --bar-clear from the bar\'s own bottom edge', () => {
    expect(pageSource).toContain('bind:this={barEl}');
    expect(pageSource).toContain('getBoundingClientRect().bottom');
    expect(pageSource).toContain('barBottom + BAR_GAP_PX');
    // ...and applies it to the body that the slots are positioned against.
    expect(pageSource).toContain('style={barClearStyle}');
  });

  it('re-measures whenever the bar or the window changes size', () => {
    // ResizeObserver: the bar grows/shrinks with its own contents (a taller
    // island, a wrapping row). visualViewport + resize: a mobile webview's
    // chrome collapsing moves everything measured against the window
    // WITHOUT resizing the bar, so the observer alone would miss it.
    expect(pageSource).toContain('new ResizeObserver');
    expect(pageSource).toContain('window.visualViewport');
    expect(pageSource).toContain("window.addEventListener('resize', measureBar)");
    // The observers are torn down with the component.
    expect(pageSource).toContain('ro?.disconnect()');
  });

  it('keeps a stylesheet constant behind the measurement', () => {
    // First paint, and any environment with no ResizeObserver: the inline
    // value has not landed yet, so the declaration must still be there --
    // the measurement overwrites it, it does not replace it.
    expect(shellCss).toMatch(/--bar-clear:\s*80px/);
    // And it no longer claims the WEB app's App.tsx is what measures this
    // desktop bar; `+page.svelte` does, and the comment should say so.
    expect(shellCss).not.toContain('App.tsx MEASURES the bar');
  });
});
