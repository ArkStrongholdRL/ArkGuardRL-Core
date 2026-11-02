//! mulberry32 and UTF-16 child-seed derivation; arithmetic matches JavaScript imul/uint32.
#[derive(Clone, Debug)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self(if seed == 0 { 0x9e3779b9 } else { seed })
    }

    /// Restore state verbatim, including zero. Unlike construction, this never reseeds.
    pub fn from_state(state: u32) -> Self {
        Self(state)
    }
    pub fn state(&self) -> u32 {
        self.0
    }

    pub fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x6d2b79f5);
        let mut t = (self.0 ^ (self.0 >> 15)).wrapping_mul(self.0 | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        t ^ (t >> 14)
    }

    pub fn next_f64(&mut self) -> f64 {
        self.next_u32() as f64 / 4294967296.0
    }
    pub fn int(&mut self, n: u32) -> u32 {
        (self.next_f64() * n as f64).floor() as u32
    }
    pub fn range(&mut self, a: f64, b: f64) -> f64 {
        a + self.next_f64() * (b - a)
    }
    pub fn chance(&mut self, probability: f64) -> bool {
        self.next_f64() < probability
    }

    pub fn pick<'a, T>(&mut self, values: &'a [T]) -> Option<&'a T> {
        if values.is_empty() {
            None
        } else {
            Some(&values[(self.next_f64() * values.len() as f64).floor() as usize])
        }
    }

    pub fn shuffle<T>(&mut self, values: &mut [T]) {
        for i in (1..values.len()).rev() {
            let j = (self.next_f64() * (i + 1) as f64).floor() as usize;
            values.swap(i, j);
        }
    }

    pub fn weighted(&mut self, weights: &[f64]) -> Option<usize> {
        // JavaScript Math.max propagates NaN; f64::max alone would discard it.
        let nonnegative = |w: f64| if w.is_nan() { w } else { w.max(0.0) };
        let total = weights.iter().fold(0.0, |sum, &w| sum + nonnegative(w));
        if total <= 0.0 || weights.is_empty() {
            return None;
        }
        let mut remaining = self.next_f64() * total;
        for (i, &weight) in weights.iter().enumerate() {
            remaining -= nonnegative(weight);
            if remaining < 0.0 {
                return Some(i);
            }
        }
        Some(weights.len() - 1)
    }
}

pub fn derive_seed(seed: u32, salt: &str) -> u32 {
    let mut h = seed ^ 0x85ebca6b;
    for unit in salt.encode_utf16() {
        h = (h ^ unit as u32).wrapping_mul(0x9e3779b1);
        h ^= h >> 13;
    }
    h
}

#[derive(Clone, Debug)]
pub struct MatchRng {
    pub setup: Rng,
    pub shop: Rng,
    pub waves: Rng,
    pub draft: Rng,
    pub bots: Rng,
    pub meta: Rng,
}

impl MatchRng {
    pub fn new(seed: u32) -> Self {
        let seed = if seed == 0 { 1 } else { seed };
        let stream = |name| Rng::new(derive_seed(seed, name));
        Self {
            setup: stream("setup"),
            shop: stream("shop"),
            waves: stream("waves"),
            draft: stream("draft"),
            bots: stream("bots"),
            meta: stream("meta"),
        }
    }
}
