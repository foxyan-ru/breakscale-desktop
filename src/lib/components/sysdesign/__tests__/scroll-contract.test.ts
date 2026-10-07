import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { render } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ArchitectureEditor from '../ArchitectureEditor.svelte';
import LowLevelEditor from '../LowLevelEditor.svelte';
import ExportPanel from '../ExportPanel.svelte';
import { sysdesignStore } from '$lib/state/sysdesign.svelte';
import type { SystemDesignDoc } from '$lib/domain';

/**
 * The system-design dialog did not scroll when the wheel was over the tab
 * panels: each panel root was its own `.scroll` container with
 * `height: 100%`, which never overflows against the fit-content card, so
 * `overscroll-behavior: contain` swallowed the gesture before `.sd-body`
 * (the dialog's one intended scroller) ever saw it. These tests pin the
 * fix: panel roots are plain blocks, and the single-scroller contract in
 * +page.svelte keeps the card clamped to the viewport.
 */
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve(null)) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));

const doc: SystemDesignDoc = {
  id: 'doc-1',
  name: 'Test design',
  schemaVersion: 1,
  createdAt: '2026-01-01T00:00:00Z',
  updatedAt: '2026-01-01T00:00:00Z',
  topology: { nodes: [], edges: [] },
  architecture: {
    summary: '',
    components: [],
    dataFlows: [],
    externalDependencies: [],
    qualityAttributes: {
      scalability: '',
      reliability: '',
      security: '',
      observability: '',
      cost: '',
    },
  },
  lowLevel: {
    apis: [],
    entities: [],
    sequences: [],
    stateMachines: [],
    deployment: {
      environment: 'test',
      vendor: null,
      region: null,
      resources: [],
      notes: '',
    },
  },
};

/** Render a panel with a real doc so the editor root (not the empty state)
 *  is the element under test, and assert the root is not a scroll container. */
function expectPlainBlock(root: Element | null, expectedClass: string): void {
  expect(root).not.toBeNull();
  expect(root).toHaveClass(expectedClass);
  expect(root).not.toHaveClass('scroll');
}

describe('sysdesign panels own no nested scroller', () => {
  beforeEach(() => {
    sysdesignStore.doc = doc;
  });

  afterEach(() => {
    sysdesignStore.doc = null;
  });

  it('ArchitectureEditor renders .arch-editor as a plain block', () => {
    const { container } = render(ArchitectureEditor);
    expectPlainBlock(container.firstElementChild, 'arch-editor');
  });

  it('LowLevelEditor renders .ld-editor as a plain block', () => {
    const { container } = render(LowLevelEditor);
    expectPlainBlock(container.firstElementChild, 'ld-editor');
  });

  it('ExportPanel renders .export-panel as a plain block', () => {
    const { container } = render(ExportPanel);
    expectPlainBlock(container.firstElementChild, 'export-panel');
  });
});

describe('dialog single-scroller contract (+page.svelte source)', () => {
  // The card clamp and .sd-body rule live in the route's <style>, which
  // jsdom cannot cascade -- assert the source contract directly instead.
  // Resolve from the vitest cwd (the package root): under the jsdom
  // environment `import.meta.url` is not a file: URL, so fileURLToPath
  // rejects it. No comment stripping: comments contain glob paths like
  // `shell/*.svelte` whose `/*` opens a bogus match and a naive
  // /* ... */ strip swallows the markup between a comment and the next
  // close. Instead the rule regexes below are specific enough (`.name {`)
  // that no comment in the file can satisfy them -- each rule is unique.
  const pageSource = readFileSync(resolve(process.cwd(), 'src/routes/+page.svelte'), 'utf8');

  it('clamps the card to the viewport instead of a percentage height', () => {
    const cardRule = pageSource.match(/\.sd-card\s*\{[^}]*\}/)?.[0] ?? '';
    expect(cardRule).toMatch(/max-height:\s*min\(720px,\s*calc\(100vh - var\(--sp-5\) \* 2\)\)/);
    // The card itself must never become a second scroll container.
    expect(cardRule).not.toMatch(/overflow\s*:/);
  });

  it('keeps .sd-body as the one scroll container of the dialog', () => {
    expect(pageSource).toMatch(/class="sd-body scroll"/);
    const bodyRule = pageSource.match(/\.sd-body\s*\{[^}]*\}/)?.[0] ?? '';
    expect(bodyRule).toMatch(/overflow-y:\s*auto/);
  });
});
