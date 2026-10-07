import { render, fireEvent } from '@testing-library/svelte';
import { tick } from 'svelte';
import type { ComponentProps } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import TrafficControl from '../TrafficControl.svelte';
import type { SystemStats } from '$lib/domain';

/**
 * Regression tests for Bug 1: the desktop header never had this control.
 * The bar carried two text buttons where the web app puts the offered-load
 * slider and the readouts that show what a drag did (p99, goodput, errors,
 * dropped), so the one input a student drives simply did not exist, and
 * `shell/Shortcuts.svelte` advertised an `S` step key nothing handled.
 *
 * TrafficControl is props-only by design (see its header), so these render
 * it in jsdom with spies instead of stores -- no Tauri bridge is reachable
 * from this file either way, and the mocks below keep it that way for the
 * transitive imports.
 */
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve(null)) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));

type TrafficProps = ComponentProps<typeof TrafficControl>;

function systemStats(over: Partial<SystemStats> = {}): SystemStats {
  return {
    timeMs: 0,
    offeredRps: 0,
    goodputRps: 0,
    errorRate: 0,
    p50: 0,
    p95: 0,
    p99: 0,
    totalRequests: 0,
    totalFailed: 0,
    ...over,
  };
}

function renderControl(overrides: Partial<TrafficProps> = {}) {
  const onRpsChange = vi.fn(() => {});
  const onToggleRun = vi.fn(() => {});
  const onStep = vi.fn(() => {});
  const onReset = vi.fn(() => {});
  const props: TrafficProps = {
    rps: 100,
    onRpsChange,
    running: false,
    onToggleRun,
    onStep,
    onReset,
    system: systemStats(),
    lost: 0,
    empty: false,
    noTrafficSource: false,
    ...overrides,
  };
  return { onRpsChange, onToggleRun, onStep, onReset, ...render(TrafficControl, { props }) };
}

function sliderOf(container: Element): HTMLInputElement {
  const input = container.querySelector('#traffic-rps');
  if (!input) throw new Error('traffic slider did not render');
  return input as HTMLInputElement;
}

describe('offered load control', () => {
  it('says there is no traffic source instead of showing a zero, and disables the slider', () => {
    // Distinct from an empty canvas: components exist (so the readouts
    // still render), there is just no client to write `rps` onto. Without
    // the disabled state the slider snapped back under the pointer with no
    // reason given; "0 requests / sec" would also claim a measurement that
    // was never taken.
    const { container } = renderControl({ noTrafficSource: true, rps: 0 });

    expect(container.querySelector('.traffic-load-none')).toHaveTextContent('No traffic source');
    expect(sliderOf(container)).toBeDisabled();
    expect(sliderOf(container).getAttribute('aria-valuetext')).toBe(
      'No traffic source on the canvas',
    );
    // The scale describes a range the slider cannot reach, so it is replaced
    // by the fix rather than sitting under a dead control.
    expect(container.querySelector('.traffic-scale')).not.toBeInTheDocument();
    expect(container.querySelector('.traffic-scale-note')).toHaveTextContent(
      'Add a client to send traffic.',
    );
    // Components are still there to report on.
    expect(container.querySelector('.traffic-metric')).toBeInTheDocument();
  });

  it('states that there is nothing to measure on a truly empty canvas', () => {
    const { container } = renderControl({ empty: true });

    expect(container.querySelector('.traffic-empty')).toHaveTextContent(
      'Nothing to measure yet.',
    );
    // No components means no readings, so the metric row is absent rather
    // than a row of zeroes.
    expect(container.querySelector('.traffic-metric')).not.toBeInTheDocument();
    // The transport still works: you can step and reset an empty canvas.
    expect(container.querySelector('.traffic-actions')).toBeInTheDocument();
  });
});

describe('slider commits', () => {
  it('keeps a drag local and commits exactly once, on pointer-up', async () => {
    const { container, onRpsChange } = renderControl({ rps: 100 });

    await fireEvent.input(sliderOf(container), { target: { value: '0' } });
    await tick();
    // `input` is the drag: the readout tracks the pointer, nothing is sent.
    expect(onRpsChange).not.toHaveBeenCalled();
    // Asserted on the figure itself, not the readout wrapper: the wrapper
    // also holds the "requests / sec" unit, so a substring check on it
    // would pass on the unit alone.
    expect(container.querySelector('.traffic-load-readout .num-lg')?.textContent).toBe('1');

    await fireEvent.change(sliderOf(container), { target: { value: '0' } });
    await tick();
    expect(onRpsChange).toHaveBeenCalledTimes(1);
    // Endpoints of the exponential track: position 0 is RPS_MIN (1), the
    // far end is RPS_MAX (5000) -- the range the web control publishes.
    expect(onRpsChange).toHaveBeenCalledWith(1);

    await fireEvent.change(sliderOf(container), { target: { value: '1000' } });
    await tick();
    expect(onRpsChange).toHaveBeenCalledTimes(2);
    expect(onRpsChange).toHaveBeenLastCalledWith(5000);
  });
});

describe('transport', () => {
  it('labels each button with what it will do and calls back', async () => {
    const { container, onToggleRun, onStep, onReset } = renderControl();

    // Named group, so a screen reader announces the cluster once instead of
    // three orphaned buttons.
    expect(container.querySelector('.traffic-actions')).toHaveAttribute(
      'aria-label',
      'Simulation transport',
    );
    expect(container.querySelector('.traffic-actions')).toHaveAttribute('role', 'group');

    await fireEvent.click(container.querySelector('.transport-toggle')!);
    expect(onToggleRun).toHaveBeenCalledTimes(1);

    await fireEvent.click(container.querySelector('.traffic-actions button:nth-child(2)')!);
    expect(onStep).toHaveBeenCalledTimes(1);

    await fireEvent.click(container.querySelector('.traffic-actions button:nth-child(3)')!);
    expect(onReset).toHaveBeenCalledTimes(1);
  });

  it('flips the primary button between Play and Pause with `running`', () => {
    const paused = renderControl({ running: false });
    expect(paused.container.querySelector('.transport-toggle')).toHaveAttribute(
      'aria-label',
      'Play',
    );

    const running = renderControl({ running: true });
    expect(running.container.querySelector('.transport-toggle')).toHaveAttribute(
      'aria-label',
      'Pause',
    );
  });
});

describe('readouts', () => {
  it('renders the sentinel for p99 while nothing is completing', () => {
    // p99 0ms beside "Errors 100%" reads as "instantly fast" in the middle
    // of a total outage. No data must render as the sentinel, never as a
    // fake zero -- a component with no meaningful value for a metric shows
    // something else (AGENTS.md).
    const { container } = renderControl({ system: systemStats({ p99: 0, goodputRps: 0 }) });

    expect(container.querySelector('.num-hero')).toHaveTextContent('n/a');
  });

  it('shows a real zero latency once work is actually completing', () => {
    const { container } = renderControl({ system: systemStats({ p99: 0, goodputRps: 50 }) });

    expect(container.querySelector('.num-hero')).toHaveTextContent('0ms');
  });

  it('always renders the three secondary metrics, in their fixed order', () => {
    const { container } = renderControl({ system: systemStats({ goodputRps: 40, errorRate: 0.02 }) });

    const labels = [...container.querySelectorAll('.traffic-metric-group .traffic-metric')].map(
      (el) => el.textContent ?? '',
    );
    expect(labels).toHaveLength(3);
    expect(labels[0]).toContain('Goodput');
    expect(labels[1]).toContain('Errors');
    expect(labels[2]).toContain('Dropped');
  });

  it('tones Dropped only while traffic is actually failing', () => {
    const healthy = renderControl({ lost: 0 });
    const healthyDropped = [...healthy.container.querySelectorAll('.traffic-metric')].find((el) =>
      (el.textContent ?? '').includes('Dropped'),
    );
    expect(healthyDropped).toBeInTheDocument();
    expect(healthyDropped!.querySelector('.is-danger')).not.toBeInTheDocument();

    const failing = renderControl({ lost: 12 });
    const failingDropped = [...failing.container.querySelectorAll('.traffic-metric')].find((el) =>
      (el.textContent ?? '').includes('Dropped'),
    );
    expect(failingDropped!.querySelector('.is-danger')).toBeInTheDocument();
  });
});
