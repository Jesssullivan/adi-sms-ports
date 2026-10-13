# Benches

## x86_64, 2x Xeon E5-2630 v3, 32 threads (shared host)

Commit `db9a1fb8fbe6`, adi-sms `5242cd0976bf`; cpu Intel(R) Xeon(R) CPU E5-2630 v3 @ 2.40GHz; cores 32; load 8.2; os Linux x86_64; python 3.15.0; ghc 9.10.3; rustc 1.98.1.

- ceiling: `--n 100000 --mean-us 0 --backoff-us 0`
- realistic: `--n 20000 --senders 1000 --capacity 1000 --mean-us 100000`

| config | runner | wall_s | min_s | sd_s | msg/s = N / wall_s | in-run msg/s, 1 run | rss_mib |
|---|---|---:|---:|---:|---:|---:|---:|
| ceiling | py | 12.8 | 12.6 | 0.292 | 7,802 | 8,004 | 21.8 |
| ceiling | rs | 0.107 | 0.0926 | 0.00829 | 937,699 | 1,023,140 | 18 |
| ceiling | hs | 0.697 | 0.685 | 0.00804 | 143,501 | 154,699 | 18 |
| ceiling | tb | 7.49 | 7.12 | 0.527 | 13,351 | 13,759 | 18 |
| realistic | py | 4.03 | 3.84 | 0.128 | 4,963 | 5,401 | 41.3 |
| realistic | rs | 2.7 | 2.7 | 0.00171 | 7,406 | 7,426 | 18 |
| realistic | hs | 2.72 | 2.71 | 0.00583 | 7,356 | 7,397 | 52.8 |
| realistic | tb | 2.94 | 2.91 | 0.0219 | 6,800 | 6,905 | 37.3 |

## arm64, Apple Silicon (quiet host)

An earlier core commit; cpu arm; cores 10; load 1.9; os Darwin arm64; python 3.14.7; ghc 9.10.3; rustc 1.98.1.

- ceiling: `--n 100000 --mean-us 0 --backoff-us 0`
- realistic: `--n 20000 --senders 1000 --capacity 1000 --mean-us 100000`

| config | runner | wall_s | sd_s | msg/s = N / wall_s | in-run msg/s, 1 run | rss_mib |
|---|---|---:|---:|---:|---:|---:|
| ceiling | py | 1.16 | 0.00309 | 86,143 | 94,472 | 43.8 |
| ceiling | rs | 0.091 | 0.00402 | 1,098,393 | 1,195,075 | 2.2 |
| ceiling | hs | 0.827 | 0.0538 | 120,943 | 90,577 | 59.9 |
| realistic | py | 3.16 | 0.00193 | 6,329 | 6,672 | 46.3 |
| realistic | rs | 3.23 | 0.07 | 6,190 | 6,367 | 3.1 |
| realistic | hs | 3.11 | 0.03 | 6,436 | 6,602 | 97.2 |

## x86_64, 8 cores (quiet host)

An earlier core commit; cpu Intel(R) Core(TM) i7-8550U CPU @ 1.80GHz; cores 8; load 1.2; os Linux x86_64; python 3.14.7; ghc 9.10.3; rustc 1.98.1.

- ceiling: `--n 100000 --mean-us 0 --backoff-us 0`
- realistic: `--n 20000 --senders 1000 --capacity 1000 --mean-us 100000`

| config | runner | wall_s | sd_s | msg/s = N / wall_s | in-run msg/s, 1 run | rss_mib |
|---|---|---:|---:|---:|---:|---:|
| ceiling | py | 2.53 | 0.0495 | 39,471 | 37,922 | 43.6 |
| ceiling | rs | 0.0328 | 0.0036 | 3,046,622 | 3,737,094 | 18.3 |
| ceiling | hs | 2.39 | 0.437 | 41,822 | 46,834 | 34.6 |
| realistic | py | 3.27 | 0.00535 | 6,108 | 6,606 | 45.9 |
| realistic | rs | 3.02 | 0.00295 | 6,631 | 6,644 | 18.3 |
| realistic | hs | 3.05 | 0.0207 | 6,555 | 6,603 | 61.8 |
