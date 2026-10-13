//! Port of adi-sms's src/adi_sms/{config,fate,summary}.py. Python is the reference; the pinned adi-sms's
//! spec/generated/vectors.json and spec.json judge this file (tests/port.rs).

use std::iter::Sum;
use std::ops::{Add, AddAssign};

pub const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;
pub const LEN: u64 = 1; // purpose tags: domain separation
pub const BODY: u64 = 2;
pub const LAT: u64 = 3;
pub const FAIL: u64 = 4;
pub const ARR: u64 = 5; // the core producer's interarrival gaps; tests/port.rs replays the raw draw

// Transcribed from spec/Spec.dhall (rendered to src/adi_sms/spec.json); tests/port.rs compares them.
pub const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 .";
pub const MAX_LEN: usize = 100;
/// Every Config field as (name, default, min, max).
pub const PARAMS: [(&str, u64, u64, u64); 11] = [
    ("n", 1000, 0, 1_000_000_000),
    ("seed", 1, 0, (1 << 53) - 1),
    ("senders", 10, 1, 10_000),
    ("capacity", 10, 1, 100_000),
    ("mean_us", 100_000, 0, 10_000_000),
    ("fail_ppm", 100_000, 0, 1_000_000),
    ("attempts", 1, 1, 10),
    ("backoff_us", 10_000, 0, 1_000_000),
    ("interval_ms", 1000, 100, 60_000),
    ("rate", 0, 0, 1_000_000),
    ("workers", 1, 1, 64),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub n: u64,
    pub seed: u64,
    pub senders: u64,
    pub capacity: u64,
    pub mean_us: u64,
    pub fail_ppm: u64,
    pub attempts: u64,
    pub backoff_us: u64,
    pub interval_ms: u64,
    /// The arrival rate and the process count of the core's pipeline. They parse and are bounds-checked, and
    /// a port ignores them: the total depends on neither (the fate is keyed by id).
    pub rate: u64,
    pub workers: u64,
}

impl Config {
    /// Values in PARAMS order.
    pub fn from_params(v: [u64; 11]) -> Self {
        let [n, seed, senders, capacity, mean_us, fail_ppm, attempts, backoff_us, interval_ms, rate, workers] =
            v;
        Config {
            n,
            seed,
            senders,
            capacity,
            mean_us,
            fail_ppm,
            attempts,
            backoff_us,
            interval_ms,
            rate,
            workers,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::from_params(PARAMS.map(|p| p.1))
    }
}

/// SplitMix64 finalizer: a bijection on 64-bit words that spreads every input bit.
pub fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub fn absorb(h: u64, w: u64) -> u64 {
    mix((h ^ w).wrapping_add(GOLDEN))
}

/// Message i's key: the seed and the id are absorbed in separate rounds.
pub fn key(seed: u64, i: u64) -> u64 {
    absorb(absorb(0, seed), i)
}

pub fn draw(k: u64, purpose: u64, j: u64) -> u64 {
    absorb(k, (purpose << 56) | j)
}

pub fn length(k: u64) -> usize {
    (draw(k, LEN, 0) % (MAX_LEN as u64 + 1)) as usize // U{0..100}
}

/// One message, stored inline so producing it allocates nothing.
pub struct Msg {
    pub id: u64,
    pub len: usize,
    pub body: [u8; MAX_LEN],
}

impl Msg {
    pub fn text(&self) -> &[u8] {
        &self.body[..self.len]
    }
}

/// Byte b is byte b % 8 (little-endian) of draw(k, BODY, b / 8), mapped to a symbol by its low 6 bits.
pub fn body(seed: u64, i: u64) -> Msg {
    let k = key(seed, i);
    let (len, mut body) = (length(k), [0; MAX_LEN]);
    for (b, chunk) in body[..len].chunks_mut(8).enumerate() {
        for (out, byte) in chunk.iter_mut().zip(draw(k, BODY, b as u64).to_le_bytes()) {
            *out = ALPHABET[(byte & 63) as usize];
        }
    }
    Msg { id: i, len, body }
}

pub fn latency_us(k: u64, j: u64, mean_us: u64) -> u64 {
    let u = (draw(k, LAT, j) >> 11) as f64 / 9_007_199_254_740_992.0; // exact 53-bit uniform in [0, 1)
    (-(mean_us as f64) * (-u).ln_1p()).floor() as u64 // Exp(mean) by inversion; ln_1p is libm log1p
}

pub fn fails(k: u64, j: u64, fail_ppm: u64) -> bool {
    (draw(k, FAIL, j) as u128) < ((fail_ppm as u128) << 64) / 1_000_000 // exact integer Bernoulli(p)
}

/// The commutative monoid every count lives in: any partition of the ids sums to the same Summary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub sent: u64,
    pub failed: u64,
    pub attempts: u64,
    pub time_us: u64, // service time: every attempt's latency plus backoff (no queue wait)
}

impl Summary {
    pub fn done(&self) -> u64 {
        self.sent + self.failed
    }

    /// The canonical last stdout line every runner prints (Python, Haskell and Rust alike).
    pub fn json(&self) -> String {
        let Summary { sent, failed, attempts, time_us } = self;
        format!(r#"{{"sent":{sent},"failed":{failed},"attempts":{attempts},"time_us":{time_us}}}"#)
    }
}

impl Add for Summary {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Summary {
            sent: self.sent + o.sent,
            failed: self.failed + o.failed,
            attempts: self.attempts + o.attempts,
            time_us: self.time_us + o.time_us,
        }
    }
}

impl AddAssign for Summary {
    fn add_assign(&mut self, o: Self) {
        *self = *self + o
    }
}

impl Sum for Summary {
    fn sum<I: Iterator<Item = Self>>(it: I) -> Self {
        it.fold(Self::default(), Add::add)
    }
}

/// Message i's whole fate, known before it starts: `wait` sees each attempt's wait (us) in order, and the
/// Summary is the outcome. At most A attempts; attempt j >= 1 first backs off beta * 2^(j-1).
pub fn each_attempt(c: &Config, i: u64, mut wait: impl FnMut(u64)) -> Summary {
    let (k, mut s) = (key(c.seed, i), Summary { failed: 1, ..Summary::default() });
    for j in 0..c.attempts {
        let w = if j > 0 { c.backoff_us << (j - 1) } else { 0 } + latency_us(k, j, c.mean_us);
        (s.attempts, s.time_us) = (s.attempts + 1, s.time_us + w);
        wait(w);
        if !fails(k, j, c.fail_ppm) {
            (s.sent, s.failed) = (1, 0);
            break;
        }
    }
    s
}

/// The fate with its waits as a list, for a sender that sleeps them. Retries stay with the sender.
pub fn deliver(c: &Config, i: u64) -> (Vec<u64>, Summary) {
    let mut waits = Vec::new();
    let s = each_attempt(c, i, |w| waits.push(w));
    (waits, s)
}

/// The pure fold over ids [a, b): run_range(a, c) == run_range(a, b) + run_range(b, c). It skips the
/// list, so a message allocates nothing.
pub fn run_range(c: &Config, a: u64, b: u64) -> Summary {
    (a..b).map(|i| each_attempt(c, i, |_| {})).sum()
}

/// run_range on `chunks` OS threads, one contiguous slice of [a, b) each. The monoid makes any split
/// exact: par_range(c, a, b, k) == run_range(c, a, b) for every k (tests/port.rs).
pub fn par_range(c: &Config, a: u64, b: u64, chunks: u64) -> Summary {
    let k = chunks.max(1);
    let cut = move |j: u64| a + ((b - a) as u128 * j as u128 / k as u128) as u64;
    std::thread::scope(|s| {
        let parts: Vec<_> = (0..k).map(|j| s.spawn(move || run_range(c, cut(j), cut(j + 1)))).collect();
        parts.into_iter().map(|p| p.join().expect("a fold thread panicked")).sum()
    })
}
