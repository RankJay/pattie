//! The scripts' choices, from one seed (xorshift64*, as `tests/ring_stress.rs`).

use super::signal::splitmix;

pub(super) struct Rng(u64);

impl Rng {
    pub(super) fn new(seed: u64) -> Rng {
        Rng(splitmix(seed) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number below `n` (`n` > 0).
    pub(super) fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).unwrap_or(0)
    }

    /// True `per_mille` times in 1,000.
    pub(super) fn chance(&mut self, per_mille: u64) -> bool {
        self.next() % 1_000 < per_mille
    }

    pub(super) fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }

    /// A size from 0 to `max`: half the time one of `marks` or a neighbour (the
    /// edges: 1 sample, a packet, the lead, the capacity, each ± 1), else any.
    pub(super) fn size(&mut self, marks: &[usize], max: usize) -> usize {
        if self.chance(500) {
            (self.pick(marks) + self.below(3))
                .saturating_sub(1)
                .min(max)
        } else {
            self.below(max + 1)
        }
    }
}
