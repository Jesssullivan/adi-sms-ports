//! `sms-rs consume`: M sender tasks on adi-sms's Valkey stream, beside the Python `sms consume`.
//! Task k is consumer `<name>-<k>` of group "senders" and runs the core's loop: a lapsed lease first
//! (XAUTOCLAIM), else a new entry (XREADGROUP); send with the fate; commit with the core's script.
//! Every min(T, lease/3) the consume loop renews each entry in hand (XCLAIM JUSTID), as the core's does.
use crate::{Config, Slot, Summary, deliver, sleep_us};
use redis::{AsyncConnectionConfig, Client, RedisResult, Script, aio::MultiplexedConnection as Conn, cmd};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::signal::unix::{SignalKind, signal};
use tokio::time::{Instant, MissedTickBehavior, interval};

/// Ack and count in one script, byte for byte the core's src/adi_sms/commit.lua (`just conform` diffs
/// them): the count happens only when the ack is new, so a redelivered entry counts once, and it returns
/// 0 then. Script runs it with EVALSHA, and loads it once on NOSCRIPT, like redis-py's register_script.
const COMMIT: &str = include_str!("../commit.lua");

/// One sender's own Summary and the stream id of the entry in its hand, which the consume loop renews.
#[derive(Default)]
struct Seat(Slot, Mutex<Option<String>>);

type Entry = (String, Vec<String>); // stream id, [field, value, ...]

/// The first entry's stream id and its `id` field.
fn first(entries: Vec<Entry>) -> Option<(String, u64)> {
    let (sid, f) = entries.into_iter().next()?;
    let id = f.chunks(2).find(|p| p[0] == "id").and_then(|p| p[1].parse().ok());
    Some((sid, id.expect("an entry without an int id")))
}

async fn take(c: &mut Conn, me: &str, lease_ms: u64) -> RedisResult<Option<(String, u64)>> {
    let mut claim = cmd("XAUTOCLAIM");
    claim.arg(&["sms", "senders", me]).arg(lease_ms).arg(&["0-0", "COUNT", "1"]);
    let (_, claimed, _): (String, Vec<Entry>, Vec<String>) = claim.query_async(c).await?;
    if let Some(m) = first(claimed) {
        return Ok(Some(m)); // a redelivery: its last consumer held it past the lease
    }
    let mut read = cmd("XREADGROUP");
    read.arg(&["GROUP", "senders", me, "COUNT", "1", "BLOCK", "1000", "STREAMS", "sms", ">"]);
    let got: Option<Vec<(String, Vec<Entry>)>> = read.query_async(c).await?;
    Ok(got.and_then(|r| r.into_iter().next()).and_then(|(_, es)| first(es)))
}

/// One sender until GET stop is "1" or SIGTERM sets `term`; the message in hand always commits.
async fn sender(
    mut c: Conn,
    cfg: Config,
    me: String,
    lease_ms: u64,
    term: Arc<AtomicBool>,
    seats: Arc<[Seat]>,
    k: usize,
) -> RedisResult<()> {
    let commit = Script::new(COMMIT); // its sha1, computed once per sender
    while !term.load(Relaxed) {
        let Some((sid, id)) = take(&mut c, &me, lease_ms).await? else {
            match cmd("GET").arg("stop").query_async::<Option<String>>(&mut c).await?.as_deref() {
                Some("1") => break,
                _ => continue,
            }
        };
        *seats[k].1.lock().unwrap() = Some(sid.clone());
        let (waits, s) = deliver(&cfg, id); // the seed is the config's, never the entry's
        for w in waits {
            sleep_us(w).await;
        }
        let mut call = commit.key("sms");
        call.key("totals").arg(&sid).arg(&[s.sent, s.failed, s.attempts, s.time_us]);
        let new = call.invoke_async::<i64>(&mut c).await? == 1; // 0: another sender committed it first
        *seats[k].1.lock().unwrap() = None;
        if new {
            *seats[k].0.0.lock().unwrap() += s;
        }
    }
    Ok(())
}

/// Run cfg.senders consumers on the stream at `queue` (host:port) until stop is "1" or SIGTERM. Every
/// min(T, lease/3) and at the end: XCLAIM JUSTID on each entry in hand, so a live sender's lease never runs
/// out; HSET snap:<name>-<k> per sender; `tick(t, sum)`. Returns this process's Summary and whether
/// SIGTERM ended it (the caller exits 143 then, 0 on stop).
pub async fn consume(
    cfg: Config,
    queue: &str,
    name: &str,
    lease_ms: u64,
    mut tick: impl FnMut(f64, Summary),
) -> RedisResult<(Summary, bool)> {
    let client = Client::open(format!("redis://{queue}/0"))?;
    let conf = AsyncConnectionConfig::new().set_response_timeout(None); // BLOCK 1000 outlasts the default 500 ms
    let mut c = client.get_multiplexed_async_connection_with_config(&conf).await?;
    match cmd("XGROUP").arg(&["CREATE", "sms", "senders", "0", "MKSTREAM"]).query_async::<()>(&mut c).await {
        Err(e) if e.code() != Some("BUSYGROUP") => return Err(e), // BUSYGROUP: someone made it first
        _ => {}
    }
    let term = Arc::new(AtomicBool::new(false));
    let mut sigterm = signal(SignalKind::terminate())?;
    let flag = term.clone();
    tokio::spawn(async move { sigterm.recv().await.map(|()| flag.store(true, Relaxed)) });
    let names: Vec<String> = (0..cfg.senders).map(|k| format!("{name}-{k}")).collect();
    let seats: Arc<[Seat]> = names.iter().map(|_| Seat::default()).collect();
    let mut tasks = tokio::task::JoinSet::new();
    for (k, me) in names.iter().enumerate() {
        let mine = client.get_multiplexed_async_connection_with_config(&conf).await?; // a BLOCK holds its connection
        tasks.spawn(sender(mine, cfg, me.clone(), lease_ms, term.clone(), seats.clone(), k));
    }
    let period = Duration::from_micros((cfg.interval_ms * 1000).min(lease_ms * 1000 / 3).max(1));
    let (t0, mut every) = (Instant::now(), interval(period));
    every.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        let done = tokio::select! {
            r = tasks.join_next() => match r {
                Some(r) => { r.expect("a sender panicked")?; continue }
                None => true,
            },
            _ = every.tick() => false,
        };
        let mut p = redis::pipe();
        for (me, seat) in names.iter().zip(seats.iter()) {
            if let Some(sid) = seat.1.lock().unwrap().clone() {
                p.cmd("XCLAIM").arg(&["sms", "senders", me.as_str(), "0", &sid, "JUSTID"]).ignore(); // idle time 0
            }
            let Summary { sent, failed, attempts, time_us } = *seat.0.0.lock().unwrap();
            let fields = [("sent", sent), ("failed", failed), ("attempts", attempts), ("time_us", time_us)];
            p.cmd("HSET").arg(format!("snap:{me}")).arg(&fields).ignore();
        }
        p.query_async::<()>(&mut c).await?;
        let total = seats.iter().map(|s| *s.0.0.lock().unwrap()).sum();
        tick(t0.elapsed().as_secs_f64(), total);
        if done {
            return Ok((total, term.load(Relaxed)));
        }
    }
}
