import { render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it } from 'vitest';
import Tooltip from '$lib/components/shell/Tooltip.svelte';
import { dismissError, pushError, uiStore } from '../ui.svelte';

/**
 * Smoke test for the frontend test setup: it exercises the SvelteKit vite
 * plugin (the `$lib` alias and `.svelte` / `.svelte.ts` compilation), the
 * jsdom environment, and the jest-dom matchers from `src/setupTests.ts`.
 */
describe('ui store', () => {
  beforeEach(() => {
    uiStore.errors = [];
  });

  it('pushError appends a toast with a unique id', () => {
    pushError('disk full');
    pushError('sync failed');

    expect(uiStore.errors.map((e) => e.message)).toEqual(['disk full', 'sync failed']);
    expect(uiStore.errors[0].id).not.toBe(uiStore.errors[1].id);
    expect(uiStore.errors[0].id).toMatch(/^err/);
  });

  it('dismissError removes only the matching toast', () => {
    pushError('first');
    pushError('second');
    const [first] = uiStore.errors;

    dismissError(first.id);

    expect(uiStore.errors).toHaveLength(1);
    expect(uiStore.errors[0].message).toBe('second');
  });

  it('starts on the canvas view with the default panel sizes', () => {
    expect(uiStore.activeView).toBe('canvas');
    expect(uiStore.railW).toBe(224);
  });
});

describe('component rendering', () => {
  it('renders a Svelte component through the $lib alias', () => {
    const { container } = render(Tooltip);

    expect(container.querySelector('.sr-only')).toBeInTheDocument();
  });
});
