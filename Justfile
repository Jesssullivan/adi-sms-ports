# adi-sms-ports: every task is a recipe here. `just` lists them; `just check` is the gate.
# Enter the toolchain first with `nix develop`: it sets ADI_SMS to the pinned adi-sms source.

set shell := ["bash", "-euo", "pipefail", "-c"]
set script-interpreter := ["bash", "-euo", "pipefail"]

default:
    @just --list --unsorted

# The pinned Haskell model and the Rust port replay adi-sms's golden vectors (their builds do); then py and
# each port at every thread count must print `sms fold`'s Summary byte for byte over bench/conformance.args.
# rs-T runs tokio on T worker threads; hs-T runs GHC on T capabilities (+RTS -N<T>). The fate is keyed by id,
# so no schedule or thread count can change the total. RUNNERS='rs hs' and THREADS='1 8' pick a subset.
# The streams row runs the pod level on one machine: a local valkey, the core's `sms produce` and `sms monitor`,
# two Python `sms consume` and one `sms-rs consume` on one stream; the totals hash must equal `sms fold`.
[script]
conform:
    : "${ADI_SMS:?conform: run inside nix develop}"; runners=${RUNNERS:-py rs hs streams}; threads=${THREADS:-1 2 8}
    out() { nix build --no-link --print-out-paths ".#$1"; }
    echo "conform: adi-sms $(jq -r '.nodes["adi-sms"].locked.rev' flake.lock)"
    py=$(out sms)
    cat "$(out model)/log" # the model's build replayed the vectors
    py_run() { "$py/bin/sms" run "$@" | jq -c "del(.check)"; } # sms run adds "check": its own run == fold
    declare -A cmd; names=()
    for r in $runners; do
      case $r in
        py) cmd[py]=py_run; names+=(py) ;;
        rs) bin=$(out sms-rs)/bin/sms-rs
          for t in $threads; do cmd[rs-$t]="env TOKIO_WORKER_THREADS=$t $bin"; names+=("rs-$t"); done ;;
        hs) bin=$(out sms-hs)/bin/sms-hs
          for t in $threads; do cmd[hs-$t]="$bin +RTS -N$t -RTS"; names+=("hs-$t"); done ;;
        streams) ;;
        *) echo "conform: unknown runner '$r' (want py, rs, hs or streams)" >&2; exit 2 ;;
      esac
    done
    err=$(mktemp); trap 'rm -f "$err"' EXIT; fail=0
    while read -r cfg; do
      [[ -z $cfg || $cfg == \#* ]] && continue
      want=$("$py/bin/sms" fold $cfg)
      for r in "${names[@]}"; do
        got=$(${cmd[$r]} $cfg 2>"$err" | tail -n 1) && st=0 || st=$?
        if [[ $st == 0 && $got == "$want" ]]; then echo "ok   $r $got <- $cfg"; continue; fi
        echo "FAIL $r (exit $st) want $want got '$got' <- $cfg"; tail -n 3 "$err"; fail=1
      done
    done < bench/conformance.args
    for r in "${names[@]}"; do # ASCII digits only, as the core's decimal(): every runner must refuse "+1"
      if ${cmd[$r]} --n +1 >/dev/null 2>&1; then echo "FAIL $r accepted --n +1"; fail=1; else echo "ok   $r rejects --n +1"; fi
    done
    [[ " $runners " == *" streams "* ]] || exit $fail
    # The commit script is one file in the core; the Rust port embeds a copy, which must not drift
    diff -u "$ADI_SMS/src/adi_sms/commit.lua" rust/commit.lua || { echo "FAIL streams: rust/commit.lua drifted"; exit 1; }
    cfg="--n 600 --senders 2 --capacity 20 --mean-us 2000 --fail-ppm 200000 --attempts 3 --backoff-us 500 --interval-ms 200 --seed 5"
    port=$((20000 + RANDOM % 20000)); q=127.0.0.1:$port; cli() { valkey-cli -p "$port" "$@"; }
    valkey-server --port "$port" --bind 127.0.0.1 --save "" >"$err" 2>&1 &
    until cli ping >/dev/null 2>&1; do sleep 0.05; done
    pids=(); rsbin=$(out sms-rs)/bin/sms-rs
    for name in py0 py1; do "$py/bin/sms" consume --queue "$q" --name "$name" $cfg >/dev/null 2>>"$err" & pids+=($!); done
    "$rsbin" consume --queue "$q" --name rs0 $cfg >/dev/null 2>>"$err" & pids+=($!)
    # Produce only once all six senders have joined the group, or the Python ones can take every message first
    joined() { cli xinfo consumers sms senders 2>/dev/null | grep -cxE '(py0|py1|rs0)-[01]' || true; }
    for _ in {1..600}; do [[ $(joined) == 6 ]] && break; sleep 0.05; done
    "$py/bin/sms" produce --queue "$q" $cfg 2>>"$err" & pids+=($!)
    final=$(timeout 300 "$py/bin/sms" monitor --queue "$q" $cfg 2>>"$err" | tail -n 1) && st=0 || st=$?
    cli set stop 1 >/dev/null # the monitor set it at done == N; a failed monitor must not strand the consumers
    for p in "${pids[@]}"; do wait "$p" || st=$?; done # every consumer exits 0 once stop is "1"
    got=$(printf '{"sent":%s,"failed":%s,"attempts":%s,"time_us":%s}' $(cli hmget totals sent failed attempts time_us))
    rs=0; for k in 0 1; do v=$(cli hget "snap:rs0-$k" sent); rs=$((rs + ${v:-0})); done # the Rust port took part
    cli shutdown nosave >/dev/null 2>&1 || true
    want=$("$py/bin/sms" fold $cfg)
    if [[ $st == 0 && $got == "$want" && $(jq .check <<<"$final") == true && $rs -gt 0 ]]; then
      echo "ok   streams py0 py1 rs0 (rs sent $rs) $got <- $cfg"
    else
      echo "FAIL streams (exit $st, rs sent $rs) want $want got '$got' monitor '$final' <- $cfg"; tail -n 5 "$err"; fail=1
    fi
    exit $fail

# Rust: vectors, spec constants, flags and properties (rust/tests); Haskell: haskell/Test.hs runs in the build
test:
    cargo test --manifest-path rust/Cargo.toml
    nix build --no-link .#sms-hs

# Format check and lints
lint:
    cargo fmt --manifest-path rust/Cargo.toml --check
    cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
    nixfmt --check flake.nix nix/*.nix

# Apply formatting
fmt:
    cargo fmt --manifest-path rust/Cargo.toml
    nixfmt flake.nix nix/*.nix

# LABEL names the run (`just bench release` on an idle machine); SMS_RS and SMS_HS may name built ports.
# SUB, a subdirectory of bench/results, keeps another machine's receipts (`just bench LABEL arm64`).
# Wall time, msg/s and peak RSS per runner on two configs -> bench/results/, bench/README.md, bench/chart.svg
[script]
bench label="rehearsal" sub="":
    py=$(nix build --no-link --print-out-paths .#sms)/bin/python # the pinned adi-sms environment
    export SMS_RS SMS_HS # tools/bench.py reads both; the ports build one after the other
    SMS_RS=${SMS_RS:-$(nix build --no-link --print-out-paths .#sms-rs)/bin/sms-rs}
    SMS_HS=${SMS_HS:-$(nix build --no-link --print-out-paths .#sms-hs)/bin/sms-hs}
    "$py" tools/bench.py xlang {{ quote(label) }} {{ sub }}
    "$py" tools/bench.py table
    "$(nix build --no-link --print-out-paths .#chart-py)/bin/python" tools/bench.py chart # matplotlib

# textbook/: MT19937 and TinyMT32 against their reference outputs, tinymt.ts against mt.py, the textbook sim's totals
textbook:
    python3 textbook/test_textbook.py

# textbook/tinymt.js from tinymt.ts, for index.html (a classic script, so the page opens from file://)
textbook-js:
    esbuild textbook/tinymt.ts --format=iife --global-name=TinyMT --log-level=warning --outfile=textbook/tinymt.js

# The public-safety gate and its hashed denylist: no private names or tool attribution in files or commits, no secrets
public-safe:
    python3 tools/public_safe.py
    gitleaks git --no-banner --redact --log-level warn .

# The gate: run before claiming anything is done; CI runs it after `nix flake check`
check: public-safe lint test textbook conform
