import { render, fireEvent } from '@testing-library/svelte';
import { tick, type ComponentProps } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import Glossary, { type GlossaryEntry } from '../Glossary.svelte';

/**
 * Glossary open/close/search regression tests for the "glossary not
 * working" report. The component is fully prop-driven (entries arrive as
 * a prop; +layout wires the Rust-fetched list in), so no IPC mocks are
 * needed here.
 */
const ENTRIES: GlossaryEntry[] = [
  {
    id: 'latency',
    term: 'Latency',
    short: 'Time a request takes end to end.',
    why: 'Slow responses frustrate users long before failures do.',
    category: 'latency',
  },
  {
    id: 'throughput',
    term: 'Throughput',
    short: 'Requests served per second.',
    why: 'Capacity planning starts from how much load the system absorbs.',
    category: 'throughput',
    aliases: ['rps'],
  },
  {
    id: 'circuit-breaker',
    term: 'Circuit breaker',
    short: 'Stops calls to a failing dependency.',
    why: 'Failing fast keeps one broken dependency from cascading.',
    category: 'component',
  },
];

function renderGlossary(overrides: Partial<ComponentProps<typeof Glossary>> = {}) {
  const onClose = vi.fn();
  const utils = render(Glossary, {
    props: { open: true, onClose, entries: ENTRIES, ...overrides },
  });
  return { onClose, ...utils };
}

describe('glossary sheet', () => {
  it('renders the dialog with every entry when opened', async () => {
    const { container, findByRole } = renderGlossary();

    // The sheet is gated on `mounted`, set by an $effect on open -- findBy
    // retries across that flush. Entries are located by their data-entry
    // hook, not by text: the section headings reuse the words "Latency"
    // and "Throughput" (CATEGORY_LABEL), so text queries would match twice.
    const dialog = await findByRole('dialog');
    expect(dialog).toBeInTheDocument();
    expect(container.querySelector('[data-entry="latency"]')).toBeInTheDocument();
    expect(container.querySelector('[data-entry="throughput"]')).toBeInTheDocument();
    expect(container.querySelector('[data-entry="circuit-breaker"]')).toBeInTheDocument();
  });

  it('stays out of the DOM while closed', () => {
    const { queryByRole } = renderGlossary({ open: false });

    expect(queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('filters entries from the search box', async () => {
    const { container, getByRole, findByRole } = renderGlossary();

    await findByRole('dialog');
    const search = getByRole('textbox', { name: 'Search glossary terms' });
    await fireEvent.input(search, { target: { value: 'circuit' } });
    await tick();

    expect(container.querySelector('[data-entry="circuit-breaker"]')).toBeInTheDocument();
    expect(container.querySelector('[data-entry="latency"]')).not.toBeInTheDocument();
    expect(container.querySelector('[data-entry="throughput"]')).not.toBeInTheDocument();
  });

  it('closes through the close button', async () => {
    const { onClose, getByRole, findByRole } = renderGlossary();

    await findByRole('dialog');
    await fireEvent.click(getByRole('button', { name: 'Close glossary' }));

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('closes on Escape', async () => {
    const { onClose, findByRole } = renderGlossary();

    const dialog = await findByRole('dialog');
    await fireEvent.keyDown(dialog, { key: 'Escape' });

    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
