"""The brief, textbook style, on the review's channels: a Pipeline, a Producer, M Senders and a Monitor.

The Pipeline creates every queue: the async queue of Message, a "done" mailbox from the Producer, and per Sender
an "end" mailbox and a size-1 stats mailbox. The Producer gets N and puts "done" after the last message. The
Pipeline then waits for the queue to drain and puts "end" to each Sender, which exits between messages. A Sender
replaces its stats after each message, so the Monitor reads progress every T. Nothing is decided ahead of time:
a Sender waits Exp(mean) drawn from its own TinyMT32, then draws the ack (ok with probability 1 - p); a failed ack
backs off and retries, up to A attempts. No fold gives the totals: they depend on how many messages each Sender
took, and thread scheduling sets that split.
"""

import argparse
import math
import queue
import sys
import threading
import time
from dataclasses import dataclass

from mt import TinyMT32

type Row = tuple[int, int, int, float]  # one Sender's sent, failed, attempts and busy seconds
ALPHABET = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 ."  # 64 symbols, as the core's
TABLE = bytes(ALPHABET[b & 63] for b in range(256))  # byte -> symbol; 64 divides 256, so unbiased
FLAGS = {  # the core's flags, defaults and bounds (its spec.json), so `sms run ARGS` and `sms.py ARGS` match
    "n": (1000, 0, 10**9, "N messages"),
    "seed": (1, 0, 2**53 - 1, "s: seeds every TinyMT32 stream; TinyMT32 keeps 32 bits, so s and s + 2^22 match"),
    "senders": (10, 1, 1023, "M Sender threads (the core allows 10000; 1024 streams per seed here)"),
    "capacity": (10, 1, 100_000, "C: the queue bound (put blocks while the queue holds C)"),
    "mean_us": (100_000, 0, 10**7, "mu: mean time in flight per attempt, us"),
    "fail_ppm": (100_000, 0, 10**6, "p: failure probability per attempt, parts per million"),
    "attempts": (1, 1, 10, "A: at most A attempts per message"),
    "backoff_us": (10_000, 0, 10**6, "beta: wait beta 2^(j-1) before retry j, us"),
    "interval_ms": (1000, 100, 60_000, "T: the Monitor's period, ms"),
}


@dataclass(frozen=True, slots=True)
class Message:
    id: int
    body: bytes  # the sketch's text


class Mailbox:  # a single-entry queue that keeps the latest: a put replaces the row, a read leaves it
    def __init__(self) -> None:
        self.row: Row = (0, 0, 0, 0.0)

    def put(self, row: Row) -> None:  # one reference store, atomic: a reader sees the old row or the new one
        self.row = row


def body(rng: TinyMT32) -> bytes:  # 0..100 symbols, one per byte of each 32-bit draw
    length = rng.genrand_int32() % 101  # the modulo bias is under 3e-8
    raw = b"".join(rng.genrand_int32().to_bytes(4, "little") for _ in range((length + 3) // 4))
    return raw[:length].translate(TABLE)


def producer(a: argparse.Namespace, q: queue.Queue[Message], done: queue.Queue[str]) -> None:
    rng = TinyMT32(a.seed * 1024)  # stream 0 of the seed's 1024; Sender k owns stream k + 1
    for i in range(a.n):
        q.put(Message(i, body(rng)))  # blocks while the queue holds C: backpressure
    done.put("done")  # back to the Pipeline: all N are queued


def sender(k: int, a: argparse.Namespace, q: queue.Queue[Message], end: queue.Queue[str], box: Mailbox) -> None:
    rng, (sent, failed, attempts, busy) = TinyMT32(a.seed * 1024 + k + 1), (0, 0, 0, 0.0)
    while True:
        try:
            q.get()  # between messages; a thread blocks on one queue at a time
        except queue.ShutDown:  # the Pipeline put "end", then shut the queue to wake this get
            break
        start = time.monotonic()
        for j in range(a.attempts):
            if j:
                time.sleep(a.backoff_us * 2 ** (j - 1) / 1e6)  # backoff before a retry
            time.sleep(-a.mean_us * math.log(1 - rng.genrand_float()) / 1e6)  # in flight for Exp(mean)
            attempts += 1
            if ok := rng.genrand_float() >= a.fail_ppm / 1e6:  # the ack: known only after the wait
                break
        sent, failed, busy = sent + ok, failed + (not ok), busy + time.monotonic() - start
        box.put((sent, failed, attempts, busy))  # progress for the Monitor: it replaces the unread one
        q.task_done()
    end.get()  # "end": the message in hand finished before the get
    box.put((sent, failed, attempts, busy))  # the final stats, then exit


def line(rows: list[Row], t: float) -> str:
    sent, failed, attempts, busy = (sum(col) for col in zip(*rows, strict=True))
    mean_ms, rate = 1000 * busy / max(1, sent + failed), (sent + failed) / max(t, 1e-9)
    return f"t={t:.1f}s sent={sent} failed={failed} attempts={attempts} mean_ms={mean_ms:.1f} rate={rate:.2f}"


def monitor(a: argparse.Namespace, boxes: list[Mailbox], t0: float, stop: threading.Event) -> None:
    while not stop.wait(a.interval_ms / 1000):  # wakes every T until the Pipeline stops it
        print(line([b.row for b in boxes], time.monotonic() - t0), file=sys.stderr, flush=True)


def pipeline(a: argparse.Namespace) -> list[Row]:
    q: queue.Queue[Message] = queue.Queue(a.capacity)  # the async queue
    done: queue.Queue[str] = queue.Queue(1)  # Producer -> Pipeline
    ends = [queue.Queue[str](1) for _ in range(a.senders)]  # Pipeline -> Sender k
    boxes = [Mailbox() for _ in range(a.senders)]  # Sender k -> Pipeline and Monitor
    t0, stop = time.monotonic(), threading.Event()
    senders = [threading.Thread(target=sender, args=(k, a, q, ends[k], boxes[k]), daemon=True) for k in range(a.senders)]
    mon = threading.Thread(target=monitor, args=(a, boxes, t0, stop), daemon=True)
    for t in [*senders, threading.Thread(target=producer, args=(a, q, done), daemon=True), mon]:
        t.start()
    done.get()  # the Producer has queued all N
    q.join()  # and the Senders have finished every one: the sim is done
    for end in ends:
        end.put("end")
    q.shutdown()  # wakes every Sender blocked in get(); none is in hand, since the queue drained
    for t in senders:
        t.join()
    stop.set()  # the Monitor's last wait ends now
    mon.join()
    rows = [b.row for b in boxes]  # each Sender's final stats
    print(line(rows, time.monotonic() - t0), file=sys.stderr, flush=True)
    return rows


def parse(argv: list[str] | None = None) -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0], allow_abbrev=False)
    for name, (default, _, _, help) in FLAGS.items():
        p.add_argument(f"--{name.replace('_', '-')}", type=int, default=default, help=help)
    a = p.parse_args(argv)
    for name, (_, low, high, _) in FLAGS.items():
        if not low <= getattr(a, name) <= high:
            p.error(f"--{name.replace('_', '-')}={getattr(a, name)}: want an int in [{low}, {high}]")
    return a


if __name__ == "__main__":  # the core's Summary line; time_us is the measured busy time
    sent, failed, attempts, busy = (sum(col) for col in zip(*pipeline(parse()), strict=True))
    print(f'{{"sent":{sent},"failed":{failed},"attempts":{attempts},"time_us":{round(busy * 1e6)}}}')
