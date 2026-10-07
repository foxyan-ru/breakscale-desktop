/**
 * Pointer-type-aware input decisions, kept pure so they can be unit tested.
 *
 * Field-for-field port of the PURE FUNCTIONS in `src/components/pointerInput.ts`
 * from the web app: drag thresholds, press-button routing, palm-touch
 * rejection, and the pinch-zoom state machine. No logic changed.
 *
 * The web app's version was a decision table for its own React pointerdown
 * router (`.cv-surface`, `Canvas.tsx`), which existed to work around specific
 * React synthetic-event/pointer-capture interactions (see
 * MIGRATION_PLAN.md §9 item 3 and `AGENTS.md`'s Canvas section). This port
 * keeps only the PURE geometry/decision functions -- thresholds, press
 * classification, palm detection, pinch math -- and none of that
 * React-specific wiring. `Canvas.svelte` wires these into Svelte's own
 * `onpointerdown`/`onpointermove`/`onpointerup` handlers directly, calling
 * `element.setPointerCapture(e.pointerId)` at gesture promotion the same way
 * any correct pointer-events implementation would (standard web platform
 * practice, not a React-specific hack).
 */

/* ------------------------------------------------------------------ *
 * Drag threshold
 * ------------------------------------------------------------------ */

/**
 * Screen-space distance a press may travel and still count as a click,
 * per pointer type.
 *
 * A mouse rests on a desk, so 4px separates intent from noise. A fingertip
 * or a pen tip is held in the air by a whole arm, and both jitter several
 * pixels during a deliberate tap; at 4px that tap promotes to a micro-drag,
 * the selection never happens, and the canvas feels broken in exactly the
 * lecture-hall setting it is built for. 10px is Excalidraw's number for the
 * same problem, arrived at in production.
 */
export const DRAG_THRESHOLD_MOUSE = 4;
export const DRAG_THRESHOLD_COARSE = 10;

export function dragThresholdFor(pointerType: string): number {
  return pointerType === 'touch' || pointerType === 'pen'
    ? DRAG_THRESHOLD_COARSE
    : DRAG_THRESHOLD_MOUSE;
}

/* ------------------------------------------------------------------ *
 * Press routing
 * ------------------------------------------------------------------ */

/**
 * What a pointerdown's button means.
 *
 *   'gesture'  primary press: routes by what was hit.
 *   'pan'      mouse middle button: forced pan.
 *   'none'     ignore outright. This is what keeps a pen's barrel button
 *              (button 2) and eraser tip (button 5) from being misread as a
 *              normal press, and what keeps right-click inert.
 */
export type PressAction = 'gesture' | 'pan' | 'none';

export function pressAction(button: number, pointerType: string): PressAction {
  if (button === 0) return 'gesture';
  if (button === 1 && pointerType === 'mouse') return 'pan';
  return 'none';
}

/* ------------------------------------------------------------------ *
 * Palm rejection
 * ------------------------------------------------------------------ */

/**
 * Contact size above which a touch is treated as a resting palm rather than
 * a fingertip, in CSS px on either axis.
 *
 * A fingertip reports roughly 20-35px on hardware that reports size at all;
 * a palm reports 60px and up. 48 sits between them with the margin on the
 * finger's side, because rejecting a legitimate touch is a worse failure
 * than letting an occasional palm through: a rejected finger looks like a
 * dead app, a stray palm is at worst an accidental pan the user watches
 * happen. Hardware that does not measure contact size reports width/height
 * as 0 or 1 and is never size-rejected.
 */
export const PALM_CONTACT_PX = 48;

/**
 * Whether a touch-down should be discarded as a palm.
 *
 * Two signals, either sufficient:
 *  - the pen is currently in contact: the hand holding it is on the glass,
 *    so every simultaneous touch is a palm or a knuckle (pen priority);
 *  - the contact patch is wider or taller than any fingertip.
 */
export function isPalmTouch(
  pointerType: string,
  width: number,
  height: number,
  penIsDown: boolean,
): boolean {
  if (pointerType !== 'touch') return false;
  if (penIsDown) return true;
  return Math.max(width || 0, height || 0) >= PALM_CONTACT_PX;
}

/* ------------------------------------------------------------------ *
 * Multi-pointer tracking and pinch
 * ------------------------------------------------------------------ */

export interface PointerPoint {
  x: number;
  y: number;
}

/** Live touch contacts, keyed by pointerId, in surface-relative coords. */
export type TouchMap = Map<number, PointerPoint>;

export interface PinchState {
  /** The two pointerIds the pinch is measured between. */
  a: number;
  b: number;
  /** Finger distance at pinch start. Scale targets are ratios against it. */
  d0: number;
  /** Viewport scale at pinch start. */
  k0: number;
  /** Midpoint at the previous frame; pan is its per-frame delta. */
  lastMid: PointerPoint;
}

export function midpoint(a: PointerPoint, b: PointerPoint): PointerPoint {
  return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
}

export function distance(a: PointerPoint, b: PointerPoint): number {
  return Math.hypot(b.x - a.x, b.y - a.y);
}

/**
 * Start a pinch from the first two tracked touches, or report null when
 * there are not two. The two fingers landing essentially on top of each
 * other would make every later ratio explode, so a degenerate start distance
 * is floored at 1px.
 */
export function beginPinch(touches: TouchMap, k0: number): PinchState | null {
  if (touches.size < 2) return null;
  const it = touches.entries();
  const [ia, pa] = it.next().value as [number, PointerPoint];
  const [ib, pb] = it.next().value as [number, PointerPoint];
  return {
    a: ia,
    b: ib,
    d0: Math.max(1, distance(pa, pb)),
    k0,
    lastMid: midpoint(pa, pb),
  };
}

export interface PinchFrame {
  /** Current midpoint, surface coords: the zoom pivot. */
  mid: PointerPoint;
  /** Midpoint travel since the last frame: the pan delta. */
  dx: number;
  dy: number;
  /**
   * ABSOLUTE target scale, computed from the initial distance and scale
   * rather than incrementally, so a long pinch cannot accumulate drift and
   * releasing at the starting spread always lands back on k0 exactly.
   */
  k: number;
}

/**
 * One frame of a live pinch. Null when either finger is no longer tracked.
 * The caller applies the frame and then advances `pinch.lastMid` to `mid`.
 */
export function pinchFrame(pinch: PinchState, touches: TouchMap): PinchFrame | null {
  const pa = touches.get(pinch.a);
  const pb = touches.get(pinch.b);
  if (!pa || !pb) return null;
  const mid = midpoint(pa, pb);
  const d = Math.max(1, distance(pa, pb));
  return {
    mid,
    dx: mid.x - pinch.lastMid.x,
    dy: mid.y - pinch.lastMid.y,
    k: pinch.k0 * (d / pinch.d0),
  };
}

/**
 * A pointer left the screen (pointerup or pointercancel): drop it from the
 * map, and end the pinch if it was one of the pinch's two fingers. Returns
 * the pinch that survives, which is either the same object or null. The
 * remaining finger of an ended pinch deliberately starts nothing: its
 * pending gesture was cancelled when the pinch began, and inventing a new
 * pan from its stale position would jump the viewport.
 */
export function endPointer(
  touches: TouchMap,
  pinch: PinchState | null,
  pointerId: number,
): PinchState | null {
  touches.delete(pointerId);
  if (pinch && (pinch.a === pointerId || pinch.b === pointerId)) return null;
  return pinch;
}

/* ------------------------------------------------------------------ *
 * Multi-selection group drag
 * ------------------------------------------------------------------ */

/**
 * Which OTHER members of the current selection a node press should carry
 * along with it. Null means "this press is a plain single-node grab".
 */
export interface GroupDragTargets {
  /** Other selected node ids, in topology order. */
  nodes: string[];
  /** Every selected annotation id (the grabbed id can never be one). */
  annotations: string[];
}

/**
 * Port of `Canvas.tsx`'s own promotion test (`if (sel.has(id) && sel.size > 1)`).
 *
 * A press on an ALREADY-SELECTED node of a multi-selection moves the whole
 * selection, and every selected annotation rides along so a marquee'd
 * cluster moves as one object whichever member was grabbed. A press on an
 * unselected node -- or on the lone selected node -- selects and moves only
 * that node instead, which is what the caller does with the null.
 *
 * The grabbed id itself is excluded from `nodes`: it is dragged through its
 * own overlay, and including it would apply the shared delta twice.
 */
export function groupDragTargets(
  hitId: string | null,
  selectedIds: ReadonlySet<string>,
  nodeIds: readonly string[],
  annotationIds: readonly string[],
): GroupDragTargets | null {
  if (!hitId || !selectedIds.has(hitId) || selectedIds.size <= 1) return null;
  return {
    nodes: nodeIds.filter((id) => id !== hitId && selectedIds.has(id)),
    annotations: annotationIds.filter((id) => selectedIds.has(id)),
  };
}

/**
 * The one delta every group member translates by, per axis.
 *
 * Both terms go through `place` (the drag's live snap function) BEFORE the
 * subtraction, so the grabbed node's own snapped position and the members'
 * shared delta are computed by the same rounding: a group preserves its
 * exact shape under grid snapping instead of each member being snapped
 * independently. `target` is the value about to become the grabbed node's
 * position, `origin` that node's position at pointerdown -- both in world
 * coords with the grab offset already removed.
 */
export function sharedSnappedDelta(
  target: number,
  origin: number,
  place: (v: number) => number,
): number {
  return place(target) - place(origin);
}
