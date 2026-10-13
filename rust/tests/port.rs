//! The port against the reference: the golden vectors and the spec constants, then the properties it
//! shares with Python: fold_equivalence (virtual time and real threads) and throughput_law.
//! ADI_SMS names the pinned adi-sms source at build time (`nix develop` and nix/ports.nix set it).
use proptest::prelude::*;
use serde_json::Value;
use sms::*;
use tokio::time::Instant;

const VECTORS: &str = include_str!(concat!(env!("ADI_SMS"), "/spec/generated/vectors.json"));
const SPEC: &str = include_str!(concat!(env!("ADI_SMS"), "/src/adi_sms/spec.json"));
const EXACT: bool = cfg!(target_os = "linux"); // libm log1p may differ by 1 ulp elsewhere: allow 1 us

fn u(v: &Value) -> u64 {
    v.as_u64().unwrap()
}

/// The defaults with some fields replaced, like Python's `replace(DEFAULTS, **fields)`.
fn config(fields: &Value) -> Config {
    let mut v = PARAMS.map(|p| p.1);
    for (name, x) in fields.as_object().unwrap() {
        v[PARAMS.iter().position(|p| p.0 == name).unwrap()] = u(x);
    }
    Config::from_params(v)
}

#[test]
fn vectors() {
    let v: Value = serde_json::from_str(VECTORS).unwrap();
    assert_eq!(v["schema"], "adi-sms-vectors-v2");
    for m in v["mix"].as_array().unwrap() {
        assert_eq!(mix(u(&m["in"])), u(&m["out"]));
    }
    let (cases, arr) = (v["cases"].as_array().unwrap(), v["arr"].as_array().unwrap());
    assert_eq!(cases.len(), arr.len()); // arr[n] is case n's raw interarrival draw
    for (c, a) in cases.iter().zip(arr) {
        let (seed, i) = (u(&c["seed"]), u(&c["i"]));
        let (k, m) = (key(seed, i), body(seed, i));
        assert_eq!(k, u(&c["key"]));
        assert_eq!(draw(k, ARR, 0), u(a), "{seed} {i}");
        assert_eq!((m.len, m.text()), (length(k), c["body"].as_str().unwrap().as_bytes()));
        assert_eq!(m.len as u64, u(&c["len"]));
        assert_eq!((c["lat"].as_array().unwrap().len(), c["fail"].as_array().unwrap().len()), (3, 3));
        for j in 0..3 {
            let (fail, lat) = (&c["fail"][j as usize], u(&c["lat"][j as usize]));
            assert_eq!(fails(k, j, u(&v["fail_ppm"])), fail.as_bool().unwrap(), "{seed} {i} {j}");
            let got = latency_us(k, j, u(&v["mean_us"]));
            assert!(got == lat || !EXACT && got.abs_diff(lat) <= 1, "{seed} {i} {j}: {got} != {lat}");
        }
    }
    for r in v["runs"].as_array().unwrap() {
        let s = run_range(&config(&r["cfg"]), u(&r["a"]), u(&r["b"]));
        let got = [s.sent, s.failed, s.attempts, s.time_us];
        let want: Vec<u64> = r["summary"].as_array().unwrap().iter().map(u).collect();
        assert_eq!(want.len(), 4, "{r}");
        let near = got[..3] == want[..3] && got[3].abs_diff(want[3]) <= want[2]; // 1 us per attempt
        assert!(got == *want || !EXACT && near, "{r}: got {got:?}");
    }
}

#[test]
fn spec_constants() {
    let s: Value = serde_json::from_str(SPEC).unwrap();
    assert_eq!(s["alphabet"].as_str().unwrap().as_bytes(), ALPHABET);
    assert_eq!(u(&s["max_len"]), MAX_LEN as u64);
    assert_eq!(s["params"].as_object().unwrap().len(), PARAMS.len());
    for (name, default, min, max) in PARAMS {
        let p = &s["params"][name];
        assert_eq!((u(&p["default"]), u(&p["min"]), u(&p["max"])), (default, min, max), "{name}");
    }
}

/// Virtual time: the clock jumps to the next timer whenever every task is waiting.
fn paused<F: Future>(f: F) -> F::Output {
    let mut rt = tokio::runtime::Builder::new_current_thread();
    rt.enable_time().start_paused(true).build().unwrap().block_on(f)
}

fn le(a: Summary, b: Summary) -> bool {
    a.sent <= b.sent && a.failed <= b.failed && a.attempts <= b.attempts && a.time_us <= b.time_us
}

// attempts, fail_ppm and seed span the spec; n, M, C and the times stay small so each case's virtual
// makespan stays short (the vectors and `just conform` cover the arithmetic at full scale).
prop_compose! {
    fn configs()(
        n in 0u64..2_000, senders in 1u64..64, capacity in 1u64..64, mean_us in 0u64..200_000,
        fail_ppm in prop_oneof![Just(0), Just(1_000_000), 0u64..=1_000_000], attempts in 1u64..=10,
        backoff_us in 0u64..50_000, interval_ms in 100u64..2_000, seed in 0..(1u64 << 53),
    ) -> Config {
        Config { n, seed, senders, capacity, mean_us, fail_ppm, attempts, backoff_us, interval_ms, ..Config::default() }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// fold_equivalence: the concurrent run equals the pure fold for any M, C, A, p, mu, beta and seed;
    /// monitor snapshots start empty at t = 0, never decrease, and the last one is the total.
    #[test]
    fn fold_equivalence(cfg in configs()) {
        let mut snaps = vec![];
        let total = paused(run(cfg, |_, s| snaps.push(s)));
        prop_assert_eq!(total, run_range(&cfg, 0, cfg.n));
        prop_assert_eq!(snaps.first(), Some(&Summary::default()));
        prop_assert!(snaps.windows(2).all(|w| le(w[0], w[1])));
        prop_assert_eq!(snaps.last(), Some(&total));
    }

    /// throughput_law (Graham): S/M <= T <= S/M + (1 - 1/M) max tau in virtual time. tokio rounds each
    /// sleep up to the next 1 ms, so S and every tau get 1 ms of slack per attempt.
    #[test]
    fn throughput_law(cfg in configs()) {
        let (t, s) = paused(async {
            let t0 = Instant::now();
            let s = run(cfg, |_, _| {}).await;
            (t0.elapsed().as_micros() as u64, s)
        });
        let slack = |s: Summary| s.time_us + 1_000 * s.attempts;
        let tau = (0..cfg.n).map(|i| slack(deliver(&cfg, i).1)).max().unwrap_or(0);
        let m = cfg.senders;
        prop_assert!(m * t >= s.time_us, "T = {t} us: faster than {m} senders can be");
        prop_assert!(m * t <= slack(s) + (m - 1) * tau, "T = {t} us: a sender idled while work remained");
    }
}

/// fold_equivalence on real threads: workers race on the channel and on the slots.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fold_equivalence_threads() {
    let cfg = Config {
        n: 20_000,
        senders: 256,
        capacity: 64,
        mean_us: 100,
        fail_ppm: 300_000,
        attempts: 3,
        backoff_us: 0,
        interval_ms: 100,
        seed: 42,
        ..Config::default()
    };
    assert_eq!(run(cfg, |_, _| {}).await, run_range(&cfg, 0, cfg.n));
}

/// The parallel fold equals the sequential fold for several N and chunk counts, replays the vectors'
/// runs, and gives the defaults' pinned answer (adi-sms docs/DESIGN.md).
#[test]
fn parallel_fold() {
    for n in [0, 1, 7, 1_000, 20_000] {
        let cfg = Config { n, fail_ppm: 300_000, attempts: 3, ..Config::default() };
        for chunks in [1, 2, 3, 8, 64] {
            assert_eq!(par_range(&cfg, 0, n, chunks), run_range(&cfg, 0, n), "n={n} chunks={chunks}");
        }
    }
    let v: Value = serde_json::from_str(VECTORS).unwrap();
    for r in v["runs"].as_array().unwrap() {
        let (cfg, a, b) = (config(&r["cfg"]), u(&r["a"]), u(&r["b"]));
        assert_eq!(par_range(&cfg, a, b, 8), run_range(&cfg, a, b), "{r}");
    }
    let want = r#"{"sent":893,"failed":107,"attempts":1000,"time_us":99301920}"#;
    assert!(!EXACT || par_range(&Config::default(), 0, 1_000, 8).json() == want);
}
