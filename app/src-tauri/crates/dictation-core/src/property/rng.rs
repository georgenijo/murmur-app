//! A tiny deterministic generator (splitmix64). The harness has no dependency
//! (the crate has none), so a seed is the whole input: the same seed always
//! produces the same scenario and the same schedule.

#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut rng = Self(seed ^ 0x9E37_79B9_7F4A_7C15);
        // Decorrelate adjacent seeds.
        rng.next();
        rng.next();
        rng
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..n` (`n > 0`).
    pub fn below(&mut self, n: u64) -> u64 {
        debug_assert!(n > 0);
        self.next() % n
    }

    pub fn index(&mut self, len: usize) -> usize {
        self.below(len as u64) as usize
    }

    /// `true` with probability `num / den`.
    pub fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }

    pub fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.index(items.len())]
    }

    /// A weighted pick: `weights[i]` is the weight of index `i`.
    pub fn weighted(&mut self, weights: &[u64]) -> usize {
        let total: u64 = weights.iter().sum();
        let mut roll = self.below(total.max(1));
        for (index, weight) in weights.iter().enumerate() {
            if roll < *weight {
                return index;
            }
            roll -= weight;
        }
        weights.len() - 1
    }

    /// A child generator: independent of how many draws the parent makes later.
    pub fn fork(&mut self) -> Self {
        Self::new(self.next())
    }
}
