//! Binary min-heap, ported from `src/sim/heap.ts`.
//!
//! Keyed by `time`, ties broken by insertion sequence (`seq`) so that two
//! events scheduled for the same simulated instant always pop in the order
//! they were pushed -- which is what makes replay deterministic when many
//! events land on the same millisecond.

/// Anything the heap can order: a simulated time, and an insertion sequence
/// number used to break ties deterministically. Mirrors the TS `Timed`
/// interface (`time: number; seq: number`). `time` is `f64` because
/// simulated time is fractional milliseconds; `seq` is `u64` because it is a
/// monotonically increasing insertion counter.
pub trait Timed {
    fn time(&self) -> f64;
    fn seq(&self) -> u64;
}

/// Array-based binary heap over `T: Timed`, smallest `(time, seq)` first.
pub struct MinHeap<T: Timed> {
    items: Vec<T>,
}

impl<T: Timed> MinHeap<T> {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn push(&mut self, item: T) {
        self.items.push(item);
        let mut i = self.items.len() - 1;
        while i > 0 {
            let parent = (i - 1) / 2;
            if less(&self.items[i], &self.items[parent]) {
                self.items.swap(i, parent);
                i = parent;
            } else {
                break;
            }
        }
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.items.is_empty() {
            return None;
        }
        // TS pops the array's last element first, then (if anything is
        // left) moves it to the root and sifts down, returning whatever
        // was at the root before that overwrite. Mirrored here rather than
        // popping the root directly, so the sift-down logic matches
        // exactly.
        let last = self.items.pop().unwrap();
        if self.items.is_empty() {
            // The removed last element WAS the only (and thus top) element.
            return Some(last);
        }
        let top = std::mem::replace(&mut self.items[0], last);
        let mut i = 0;
        loop {
            let l = 2 * i + 1;
            let r = l + 1;
            let mut smallest = i;
            if l < self.items.len() && less(&self.items[l], &self.items[smallest]) {
                smallest = l;
            }
            if r < self.items.len() && less(&self.items[r], &self.items[smallest]) {
                smallest = r;
            }
            if smallest == i {
                break;
            }
            self.items.swap(i, smallest);
            i = smallest;
        }
        Some(top)
    }

    pub fn peek(&self) -> Option<&T> {
        self.items.first()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }
}

impl<T: Timed> Default for MinHeap<T> {
    fn default() -> Self {
        Self::new()
    }
}

fn less<T: Timed>(a: &T, b: &T) -> bool {
    if a.time() != b.time() {
        a.time() < b.time()
    } else {
        a.seq() < b.seq()
    }
}

/// No heap-specific test file exists in `src/sim` (checked before writing
/// these): `heap.ts` is exercised only indirectly, through `engine.test.ts`
/// and friends. The tests below are therefore a fresh port of the intent,
/// not a transcription of an existing suite. They verify internal
/// correctness of this Rust implementation only -- pop order for a small
/// fixed set of `(time, seq)` pairs, and that ties are broken by `seq` --
/// not that it matches the TS heap's behaviour bit-for-bit (that would
/// require a cross-language fixture, which was not run in this session; see
/// MIGRATION_PLAN.md #9).
#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct Event {
        time: f64,
        seq: u64,
        label: &'static str,
    }

    impl Timed for Event {
        fn time(&self) -> f64 {
            self.time
        }
        fn seq(&self) -> u64 {
            self.seq
        }
    }

    #[test]
    fn ties_are_broken_by_seq() {
        let mut heap = MinHeap::new();
        heap.push(Event { time: 5.0, seq: 2, label: "b" });
        heap.push(Event { time: 5.0, seq: 1, label: "a" });
        heap.push(Event { time: 5.0, seq: 3, label: "c" });

        assert_eq!(heap.pop().unwrap().label, "a");
        assert_eq!(heap.pop().unwrap().label, "b");
        assert_eq!(heap.pop().unwrap().label, "c");
        assert!(heap.pop().is_none());
    }

    #[test]
    fn pop_until_empty_is_sorted_by_time_then_seq() {
        let mut heap = MinHeap::new();
        let pairs: [(f64, u64); 8] = [
            (3.0, 1),
            (1.0, 2),
            (4.0, 3),
            (1.0, 4),
            (5.0, 5),
            (9.0, 6),
            (2.0, 7),
            (6.0, 8),
        ];
        for (time, seq) in pairs {
            heap.push(Event { time, seq, label: "x" });
        }
        assert_eq!(heap.len(), pairs.len());

        let mut out = Vec::new();
        while let Some(e) = heap.pop() {
            out.push((e.time, e.seq));
        }
        assert!(heap.is_empty());

        let mut expected = pairs.to_vec();
        expected.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(a.1.cmp(&b.1)));
        assert_eq!(out, expected);
    }

    #[test]
    fn peek_does_not_remove() {
        let mut heap = MinHeap::new();
        heap.push(Event { time: 2.0, seq: 1, label: "only" });
        assert_eq!(heap.peek().unwrap().label, "only");
        assert_eq!(heap.len(), 1);
    }

    #[test]
    fn clear_empties_the_heap() {
        let mut heap = MinHeap::new();
        heap.push(Event { time: 1.0, seq: 1, label: "a" });
        heap.push(Event { time: 2.0, seq: 2, label: "b" });
        heap.clear();
        assert!(heap.is_empty());
        assert!(heap.pop().is_none());
    }
}
