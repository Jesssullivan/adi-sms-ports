//! sms-rs: the flags of `sms run`. Monitor lines go to stderr; the last stdout line is the canonical
//! Summary JSON, so `just conform` compares bytes. TOKIO_WORKER_THREADS sets the worker count.
//! `sms-rs consume --queue HOST:PORT --name NAME [--lease-ms L] <flags>` joins adi-sms's Valkey stream
//! (stream.rs) and exits 0 when the queue says stop, 143 after a SIGTERM.
use sms::{Config, PARAMS, Summary, consume, run};

/// ASCII digits only, as the core's decimal(): `str::parse` alone would take "+1".
fn decimal<T: std::str::FromStr>(x: &str) -> Option<T> {
    (!x.is_empty() && x.bytes().all(|b| b.is_ascii_digit())).then(|| x.parse().ok())?
}

fn parse(args: &[String]) -> Result<Config, String> {
    let mut v = PARAMS.map(|p| p.1);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let (flag, inline) = arg.split_once('=').map_or((arg.as_str(), None), |(f, x)| (f, Some(x)));
        let k = (flag.strip_prefix("--").map(|f| f.replace('-', "_")))
            .and_then(|f| PARAMS.iter().position(|p| p.0 == f))
            .ok_or(format!("unrecognized argument: {arg}"))?;
        let (name, _, lo, hi) = PARAMS[k];
        // or_else, not or: the next argument is consumed only when the value is not inline
        let x = inline
            .or_else(|| it.next().map(String::as_str))
            .ok_or(format!("{flag}: expected one argument"))?;
        v[k] = (decimal(x).filter(|x| (lo..=hi).contains(x)))
            .ok_or(format!("{name}={x}: want an int in [{lo}, {hi}]"))?;
    }
    Ok(Config::from_params(v))
}

/// Remove `flag VALUE` or `flag=VALUE` from args and return the value, if the flag is there.
fn take(args: &mut Vec<String>, flag: &str) -> Result<Option<String>, String> {
    let Some(i) = args.iter().position(|a| a.split_once('=').map_or(a.as_str(), |p| p.0) == flag) else {
        return Ok(None);
    };
    match args.remove(i).split_once('=') {
        Some((_, x)) => Ok(Some(x.into())),
        None if i < args.len() => Ok(Some(args.remove(i))),
        None => Err(format!("{flag}: expected one argument")),
    }
}

/// The Python monitor's fields: elapsed s, the Summary, mean service time per message (ms), rate (msg/s).
fn line(t: f64, s: Summary) -> String {
    let done = s.done() as f64;
    let mean = if done > 0.0 { s.time_us as f64 / done / 1e3 } else { 0.0 };
    let tput = if t > 0.0 { done / t } else { 0.0 };
    let Summary { sent, failed, attempts, time_us } = s;
    format!(
        "t={t:.2} sent={sent} failed={failed} attempts={attempts} time_us={time_us} mean_ms={mean:.2} rate={tput:.2}"
    )
}

#[tokio::main] // multi-thread: one worker per core
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        let flags: Vec<_> = PARAMS.iter().map(|p| format!("[--{} {}]", p.0.replace('_', "-"), p.1)).collect();
        let f = flags.join(" ");
        return println!(
            "usage: sms-rs {f}\n       sms-rs consume --queue HOST:PORT --name NAME [--lease-ms 30000] {f}"
        );
    }
    let fail = |e: String| -> ! {
        eprintln!("sms-rs: error: {e}");
        std::process::exit(2)
    };
    let monitor = |t, s| eprintln!("{}", line(t, s));
    let Some(mut args) = args.strip_prefix(&["consume".into()]).map(<[String]>::to_vec) else {
        let cfg = parse(&args).unwrap_or_else(|e| fail(e));
        return println!("{}", run(cfg, monitor).await.json());
    };
    let mut opt = |flag| take(&mut args, flag).unwrap_or_else(|e| fail(e));
    let (queue, name, lease) = (opt("--queue"), opt("--name"), opt("--lease-ms"));
    let (Some(queue), Some(name)) = (queue, name) else {
        fail("consume: want --queue HOST:PORT --name NAME".into())
    };
    let lease = lease.map_or(Ok(30_000), |l| decimal(&l).ok_or(format!("--lease-ms={l}: want an int")));
    let (cfg, lease) = (parse(&args).unwrap_or_else(|e| fail(e)), lease.unwrap_or_else(|e| fail(e)));
    match consume(cfg, &queue, &name, lease, monitor).await {
        Ok((s, term)) => {
            println!("{}", s.json());
            std::process::exit(if term { 143 } else { 0 }) // 143 = 128 + SIGTERM, after the flush
        }
        Err(e) => fail(format!("consume: {e}")),
    }
}

#[test]
fn flags() {
    let parse = |s: &str| parse(&s.split_whitespace().map(String::from).collect::<Vec<_>>());
    let want = Ok(Config { n: 7, mean_us: 0, ..Config::default() });
    assert_eq!(parse("--n=7 --mean-us 0"), want);
    assert_eq!(parse("--n 7 --mean-us=0"), want);
    assert!(parse("--n -1").is_err() && parse("--n").is_err() && parse("--chunk 5").is_err());
    assert!(
        parse("--n +7").is_err()
            && parse("--n=+7").is_err()
            && parse("--n 1_0").is_err()
            && parse("--n=").is_err()
    );
    let mut args: Vec<String> = "--n 7 --queue=h:1 --name a".split_whitespace().map(String::from).collect();
    assert_eq!(take(&mut args, "--queue"), Ok(Some("h:1".into())));
    assert_eq!(take(&mut args, "--name"), Ok(Some("a".into())));
    assert_eq!((take(&mut args, "--lease-ms"), args.join(" ")), (Ok(None), "--n 7".into()));
    assert!(take(&mut vec!["--name".into()], "--name").is_err());
}
