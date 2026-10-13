#!/bin/sh
# Check port binaries against the core's `sms fold`, then time them on the CPU ceiling config.
# bench/run.sh [BINARY...]: default ./sms-rs ./sms-hs. SMS names the reference command; by default uvx
# fetches the core and CPython 3.15 from GitHub. Exit 1 if any Summary differs.
# shellcheck disable=SC2086 # $sms and the configs split into words on purpose
set -u
[ $# -gt 0 ] || set -- ./sms-rs ./sms-hs
sms=${SMS:-uvx --from git+https://github.com/Jesssullivan/adi-sms sms}
# The defaults (about 10 s per binary), then retries with backoff at M = C (bench/conformance.args)
retry='--n 5000 --senders 64 --capacity 64 --mean-us 300 --fail-ppm 250000 --attempts 3 --backoff-us 200 --seed 42'
fail=0
for cfg in '' "$retry"; do
  want=$($sms fold $cfg) || exit 2
  for bin in "$@"; do
    got=$("$bin" $cfg 2>/dev/null | tail -n 1)
    if [ "$got" = "$want" ]; then echo "ok   $bin $got <- ${cfg:-defaults}"; continue; fi
    echo "FAIL $bin want $want got '$got' <- ${cfg:-defaults}"
    fail=1
  done
done
[ $fail = 0 ] || exit 1 # time only binaries that match
# No waits: the fold's work per message, as in bench/README.md
ceiling='--n 100000 --mean-us 0 --backoff-us 0'
if command -v hyperfine >/dev/null; then
  for bin in "$@"; do set -- "$@" "$bin $ceiling"; shift; done
  hyperfine -N --warmup 1 "$@" "$sms run $ceiling"
else
  for c in "$@" "$sms run"; do
    echo "time: $c $ceiling"
    time $c $ceiling >/dev/null 2>&1
  done
fi
