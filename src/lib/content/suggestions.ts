/**
 * One grounded suggestion per node kind, offered by the Inspector only once
 * that node's "spare capacity" (headroom, see `field-schema.ts`'s
 * `headroomFor`/`HAS_THROUGHPUT_CEILING`) drops below 1.0x -- the node
 * cannot keep up with what is arriving.
 *
 * Port of upstream `src/content/suggestions.ts` ("Suggest one fix for a
 * component that cannot keep up", `eb163673`, PR #71, closing issue #23).
 * Lives beside the glossary rather than in the simulation engine for the
 * same reason upstream's own history moved it there (PR #71's fourth
 * commit): nothing in `sim/` reads these strings and no script would ever
 * want them -- they are text shown to a student.
 *
 * Kinds this applies to: exactly `HAS_THROUGHPUT_CEILING` in
 * `components/inspector/field-schema.ts` -- headroom is only ever a defined
 * concept for those, so a suggestion is only ever offered for those.
 *
 * ONE suggestion per kind, never several with tradeoffs. Per the upstream PR
 * body: a plausible-sounding fix that makes things worse is worse than
 * saying nothing. In particular:
 *   - a database is NOT told to get bigger -- that specific advice is a trap
 *     the app's own challenges set up deliberately (upstream issue #23);
 *     instead it is pointed at what is reaching it.
 *   - a saturated cache is NOT told to raise its hit rate -- the hit/miss
 *     roll happens after a slot is already held (mirrors onServiceComplete
 *     in the web engine; see `src-tauri/src/sim/behaviour/` for this port's
 *     equivalent), so a hit and a miss occupy a slot for the same
 *     `serviceMs`. Hit rate changes what the cache FORWARDS, never what
 *     arrives at it, so raising it moves nothing on the node that is itself
 *     over its ceiling.
 *
 * Text copied verbatim from upstream (including its no-em-dash rule, per
 * upstream CONTRIBUTING and matched here so a future diff stays a true
 * no-op), with "instances" read as this app's per-machine count exactly as
 * upstream intends.
 */
import type { NodeKind } from '$lib/domain';

export function suggestionFor(kind: NodeKind): string | null {
  switch (kind) {
    case 'cache':
      return 'Add instances, or more slots per instance. Hit rate will not help here, because a hit and a miss occupy a slot for the same time. It changes what this cache forwards, not what it has to get through.';
    case 'db':
      return 'A database rarely gets faster by being made bigger. Look at what is reaching it instead. A cache in front of it with a higher hit rate, or fewer retries piling on load, will do more than resizing this node.';
    case 'lb':
      return 'A load balancer saturating is uncommon. Check the components behind it are not the actual limit before adding capacity here.';
    case 'sidecar':
      return 'Each hop pays this proxy a tax. Check retries and timeouts before adding capacity; a retry storm here costs more than the base load does.';
    case 'apigateway':
      return 'This is a stateless front door, so add instances to spread the load across more of them.';
    case 'worker':
      return 'Workers drain a queue at their own pace. Add instances to drain it faster.';
    case 'service':
    case 'objectstore':
    case 'coldstorage':
    case 'retryqueue':
    case 'transcoder':
    case 'edgecompute':
      return 'Add instances for more parallel slots, or lower service time if the work itself can be made faster.';
    default:
      return null;
  }
}
