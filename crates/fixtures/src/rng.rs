pub const SEED: u64 = 0x5350_494E_4553_5052;

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn for_item(book: &str, ix: u64) -> Self {
        Rng(derive(book, ix))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    pub fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + self.below(hi - lo + 1)
    }
}

pub fn derive(label: &str, ix: u64) -> u64 {
    let mut h = SEED ^ 0xCBF2_9CE4_8422_2325;
    for b in label.bytes().chain(ix.to_le_bytes()) {
        h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}
