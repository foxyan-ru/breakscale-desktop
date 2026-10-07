import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { describe, expect, it } from 'vitest';
import Glossary, { type GlossaryEntry, type GlossaryCategory } from '../Glossary.svelte';

/**
 * Data-contract test for the "glossary opens but shows no results" report.
 *
 * The panel is prop-driven; the whole runtime path is: Rust serves
 * `src-tauri/data/glossary.json` through `glossary::load()`'s typed serde
 * pass, `+layout.svelte` hands the array to `<Glossary entries=...>`, and
 * the component groups/filters it. The unit tests in glossary.test.ts pin
 * the component with hand-written fixtures; this one pins the REAL file
 * end-to-end through the component, so data drift (a renamed category, a
 * typo'd key) fails CI instead of surfacing as an empty panel.
 *
 * Read from the vitest cwd like the scroll-contract test does: under the
 * jsdom environment `import.meta.url` is not a file: URL.
 */
const raw = readFileSync(resolve(process.cwd(), 'src-tauri/data/glossary.json'), 'utf8');
const entries = JSON.parse(raw) as GlossaryEntry[];

const CATEGORIES: GlossaryCategory[] = ['latency', 'throughput', 'failure', 'capacity', 'component', 'unit'];

describe('glossary.json survives the component pipeline', () => {
  it('declares the fields the component reads, with frontend categories', () => {
    expect(entries.length).toBe(100);
    for (const entry of entries) {
      expect(entry.id, 'id').toBeTruthy();
      expect(entry.term, `term of ${entry.id}`).toBeTruthy();
      expect(entry.short, `short of ${entry.id}`).toBeTruthy();
      expect(entry.why, `why of ${entry.id}`).toBeTruthy();
      expect(CATEGORIES, `category of ${entry.id}`).toContain(entry.category);
    }
  });

  it('renders every entry without showing the empty state', async () => {
    const { container, findByRole, queryByText } = render(Glossary, {
      props: { open: true, onClose: () => {}, entries },
    });

    await findByRole('dialog');
    await tick();

    // All 100 make it through group() + the category ordering.
    expect(container.querySelectorAll('[data-entry]')).toHaveLength(100);
    // The empty result copy must not appear when nothing is filtered out.
    expect(queryByText(/^No term matches/)).not.toBeInTheDocument();
  });

  it('finds a real term through the search box', async () => {
    const { container, getByRole, findByRole } = render(Glossary, {
      props: { open: true, onClose: () => {}, entries },
    });

    await findByRole('dialog');
    const probe = entries[42];
    const search = getByRole('textbox', { name: 'Search glossary terms' });
    await fireEvent.input(search, { target: { value: probe.term } });
    await tick();

    expect(container.querySelector(`[data-entry="${probe.id}"]`)).toBeInTheDocument();
    expect(container.querySelector('[data-entry]')).not.toBeNull();
  });
});
