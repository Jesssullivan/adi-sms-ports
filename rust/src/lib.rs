//! The machine: producer -> bounded channel (C) -> M sender tasks, with a monitor every T.
//! The Rust restatement of adi-sms's src/adi_sms/pipeline.py at the thread level; its result must equal the pure fold
//! (tests/port.rs). `consume` (stream.rs) runs the same senders on a shared Valkey stream instead.
mod fate;
mod stream;
pub use fate::*;
pub use stream::consume;

use std::pin::pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep};

/// One sender's running Summary. A single writer means no contention; 128-byte alignment keeps
/// neighbours off one cache line. The monitor sums the slots: the monoid, like `sum()` across pods.
#[derive(Default)]
#[repr(align(128))]
pub(crate) struct Slot(pub(crate) Mutex<Summary>);

/// Send cfg.n messages. `tick(t, snapshot)` runs at t = 0, every interval, and once more with the total.
pub async fn run(cfg: Config, mut tick: impl FnMut(f64, Summary)) -> Summary {
    let (tx, rx) = async_channel::bounded::<Msg>(cfg.capacity as usize); // send waits while C are queued
    let slots: Arc<[Slot]> = (0..cfg.senders).map(|_| Slot::default()).collect();
    let snapshot = || slots.iter().map(|s| *s.0.lock().unwrap()).sum::<Summary>();
    let senders: Vec<_> = (0..slots.len())
        .map(|k| {
            let (rx, slots) = (rx.clone(), slots.clone());
            tokio::spawn(async move {
                // recv() errs only once the channel is closed and drained
                while let Ok(m) = rx.recv().await {
                    let (waits, s) = deliver(&cfg, m.id);
                    for w in waits {
                        sleep_us(w).await;
                    }
                    *slots[k].0.lock().unwrap() += s; // never held across an await
                }
            })
        })
        .collect();
    drop(rx); // if every sender dies, send() fails instead of waiting forever
    let mut work = pin!(async move {
        for i in 0..cfg.n {
            tx.send(body(cfg.seed, i)).await.expect("every sender died");
        }
        drop(tx); // close: senders drain the queue, then stop
        for s in senders {
            s.await.expect("a sender panicked");
        }
    });
    let (t0, period) = (Instant::now(), Duration::from_millis(cfg.interval_ms));
    tick(0.0, snapshot()); // here, not as interval's first tick: that waits for the timer, up to 1 ms
    let mut every = interval_at(t0 + period, period);
    every.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            () = &mut work => break,
            _ = every.tick() => tick(t0.elapsed().as_secs_f64(), snapshot()),
        }
    }
    let total = snapshot();
    tick(t0.elapsed().as_secs_f64(), total);
    total
}

/// tokio rounds a sleep up to the next 1 ms tick, so a zero wait would cost up to 1 ms: skip it.
pub(crate) async fn sleep_us(us: u64) {
    if us > 0 {
        sleep(Duration::from_micros(us)).await
    }
}
