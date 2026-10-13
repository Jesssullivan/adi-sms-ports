"""Cross-language benchmark -> bench/results/xlang.json (schema adi-sms-bench-v1) -> bench/README.md.
    python tools/bench.py xlang LABEL [SUB]   per runner: hyperfine wall time, in-run msg/s and peak RSS
    python tools/bench.py table               render bench/README.md from the receipts
    python tools/bench.py chart               draw bench/chart.svg from bench/results/xlang.json (matplotlib: .#chart-py)
SUB, a subdirectory of bench/results, keeps another machine's receipts; the table renders them last.
Runners: `sms run` on this interpreter (the pinned adi-sms environment; `just bench` starts it), plus $SMS_RS
and $SMS_HS, and textbook/sms.py on this interpreter. Every measured run of a port must print the fold's Summary;
the textbook has no fold, so its run must send or fail each of the N messages once. Peak RSS is the child's
wait4 rusage (what /usr/bin/time -v reports); no host name is recorded."""

import json
import os
import platform
import re
import shlex
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "bench/results"
XLANG = {  # zero waits: generation and bookkeeping (Python's CPU ceiling); then senders that mostly sleep
    "ceiling": "--n 100000 --mean-us 0 --backoff-us 0",
    "realistic": "--n 20000 --senders 1000 --capacity 1000 --mean-us 100000",
}
RUNS = 5  # hyperfine runs per runner and config, after one warm-up
MEASURED = ("haskell", "rust", "textbook", "nix", "flake.nix", "flake.lock")
Rows = list[dict[str, Any]]


def sh(*cmd: str) -> str:
    return subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()


def once(r: str, argv: list[str], args: str, want: str) -> tuple[float, float]:  # want: the last line, or N
    """One checked run: msg/s from its last monitor line (start-up excluded), and peak RSS in MiB."""
    with tempfile.TemporaryFile("w+") as f:  # stderr, then the Summary: the last line on stdout
        p = subprocess.Popen([*argv, *args.split()], stdout=f, stderr=subprocess.STDOUT)
        _, status, use = os.wait4(p.pid, 0)  # this child's own rusage, unlike RUSAGE_CHILDREN
        p.returncode = os.waitstatus_to_exitcode(status)
        f.seek(0)
        log = f.read()
    last = log.splitlines()[-1:]
    if r == "tb":  # the textbook's Summary: every message sent or failed once
        last = [str(sum(json.loads(last[0])[k] for k in ("sent", "failed")))] if last else []
    if p.returncode or last != [want]:
        sys.exit(f"bench: {r} {args}: exit {p.returncode}, want {want}, output ends {log[-300:]!r}")
    mib = use.ru_maxrss / (2**20 if sys.platform == "darwin" else 2**10)  # bytes on macOS, KiB on Linux
    return float(re.findall(r"rate=([\d.]+)", log)[-1]), round(mib, 1)


def xlang(run: dict[str, list[str]], rows: Rows) -> dict[str, str]:
    for cfg, args in XLANG.items():
        with tempfile.NamedTemporaryFile(suffix=".json") as f:
            names = [x for r, argv in run.items() for x in ("-n", r, shlex.join([*argv, *args.split()]))]
            hf = ["hyperfine", "-N", "--warmup", "1", "--runs", str(RUNS), "--export-json", f.name, *names]
            subprocess.run(hf, check=True)
            wall = {h["command"]: h for h in json.loads(Path(f.name).read_text())["results"]}
        want = sh(sys.executable, "-m", "adi_sms", "fold", *args.split())
        n = str(sum(json.loads(want)[k] for k in ("sent", "failed")))
        for r, argv in run.items():  # `sms run` ends with the Summary and "check": its run equals the fold
            tput, mib = once(r, argv, args, {"py": want[:-1] + ',"check":true}', "tb": n}.get(r, want))
            h = {"wall_s": wall[r]["mean"], "min_s": wall[r]["min"], "sd_s": wall[r]["stddev"]}
            rows.append({"config": cfg, "runner": r, **h, "msg_s": tput, "rss_mib": mib})
    return XLANG


def bench(label: str, out: Path) -> None:
    run = {"py": [sys.executable, "-m", "adi_sms", "run"]}
    run |= {r: [os.environ[v]] for r in ("rs", "hs") if (v := f"SMS_{r.upper()}") in os.environ}
    run["tb"] = [sys.executable, str(ROOT / "textbook/sms.py")]  # the core's flags and defaults
    info = Path("/proc/cpuinfo")
    cpu = re.findall(r"model name\s*:\s*(.+)", info.read_text() if info.exists() else "")
    m = dict[str, object](label=label, cpu=(cpu or [platform.processor()])[0], cores=os.cpu_count())
    m |= {"load": round(os.getloadavg()[0], 1), "os": f"{platform.system()} {platform.machine()}"}
    m["python"] = platform.python_version()
    if "hs" in run:
        m["ghc"] = re.findall(r'"GHC version", "([^"]+)"', sh(*run["hs"], "+RTS", "--info"))[0]
    if "rs" in run:
        m["rustc"] = sh("rustc", "--version").split()[1]
    rows = Rows()
    config = xlang(run, rows)
    sha = sh("git", "rev-parse", "--short=12", "HEAD") + "-dirty" * bool(sh("git", "status", "-s", *MEASURED))
    pin = json.loads((ROOT / "flake.lock").read_text())["nodes"]["adi-sms"]["locked"]["rev"][:12]
    doc = {"schema": "adi-sms-bench-v1", "commit": sha, "adi_sms": pin, "machine": m, "config": config}
    out.mkdir(parents=True, exist_ok=True)
    (out / "xlang.json").write_text(json.dumps(doc | {"rows": rows}, indent=1) + "\n")


def flags(args: str) -> dict[str, float]:
    return dict(zip(args.split()[::2], map(float, args.split()[1::2])))


def rates(d: dict[str, Any]) -> dict[tuple[str, str], tuple[float, float, float]]:
    """msg/s = N / the mean wall time, then at the mean + 1 sd and - 1 sd: hyperfine's spread, as a rate."""
    n = {c: flags(a)["--n"] for c, a in d["config"].items()}
    return {
        (r["config"], r["runner"]): tuple(n[r["config"]] / (r["wall_s"] + k * r["sd_s"]) for k in (0, 1, -1))
        for r in d["rows"]
    }


def cell(v: object) -> str:  # past 100 a whole number with separators, else three significant digits
    return (f"{v:,.0f}" if v >= 100 else f"{v:.3g}") if isinstance(v, float) else str(v)


def table() -> None:
    out, subs = ["# Benches", ""], sorted(p for p in OUT.iterdir() if p.is_dir())
    for f in (s / "xlang.json" for s in (OUT, *subs)):  # canonical machine first
        if not f.exists():
            continue
        d = json.loads(f.read_text())
        rate, head = rates(d), list(d["rows"][0])
        i = head.index("msg_s")  # N / wall_s first, then the in-run rate the receipt keeps
        head[i : i + 1] = ["msg/s = N / wall_s", "in-run msg/s, 1 run"]
        machine = "; ".join(f"{k} {cell(v)}" for k, v in d["machine"].items() if k != "label")
        at = f"Commit `{d['commit']}`, adi-sms `{d['adi_sms']}`" if "adi_sms" in d else "An earlier core commit"
        out += [f"## {d['machine']['label']}", "", f"{at}; {machine}.", ""]
        out += [*(f"- {k}: `{v}`" for k, v in d["config"].items()), ""]
        out += ["| " + " | ".join(head) + " |", "|---|---|" + "---:|" * (len(head) - 2)]
        for row in d["rows"]:
            v = list(row.values())
            v[i:i] = [rate[row["config"], row["runner"]][0]]
            out.append("| " + " | ".join(map(cell, v)) + " |")
        out.append("")
    (ROOT / "bench/README.md").write_text("\n".join(out))


RUNNERS = {"py": "py (core)", "rs": "rs (Rust)", "hs": "hs (Haskell)", "tb": "tb (textbook)"}
COLOUR = dict(zip(RUNNERS, ("#2a78d6", "#eb6834", "#1baf7a", "#eda100")))


def chart() -> None:
    """bench/chart.svg from the canonical receipt: msg/s = N / the mean wall time, whiskers at the mean ± 1 sd."""
    import matplotlib  # here, not at the top: xlang and table run in the adi-sms environment, which has no matplotlib

    matplotlib.use("svg")
    import matplotlib.pyplot as plt
    from matplotlib.ticker import EngFormatter

    plt.rcParams |= {"svg.fonttype": "none", "svg.hashsalt": "adi-sms", "font.size": 10}  # text stays text; stable ids
    d = json.loads((OUT / "xlang.json").read_text())
    rate, f = rates(d), {c: flags(a) for c, a in d["config"].items()}
    m, mu = f["realistic"]["--senders"], f["realistic"]["--mean-us"]
    titles = {
        "ceiling": f"ceiling: no waits, N = {f['ceiling']['--n']:,.0f}",
        "realistic": f"realistic: μ = {mu / 1000:g} ms, M = {m:.0f}",
    }
    fig, axes = plt.subplots(1, 2, figsize=(10, 4), layout="constrained")
    for ax, (cfg, title) in zip(axes, titles.items()):
        rs = [r for r in RUNNERS if (cfg, r) in rate]
        v, lo, hi = zip(*(rate[cfg, r] for r in rs))
        err = [[a - b for a, b in zip(v, lo)], [b - a for a, b in zip(v, hi)]]
        bars = ax.bar([RUNNERS[r] for r in rs], v, yerr=err, capsize=4, color=[COLOUR[r] for r in rs])
        ax.bar_label(bars, [f"{x:,.0f}" for x in v], padding=3)
        ax.set_title(title, loc="left")
        ax.set_ylabel("msg/s, log scale" if cfg == "ceiling" else "msg/s")
        if cfg == "ceiling":
            ax.set_yscale("log")
        else:
            ax.axhline(cap := m * 1e6 / mu, ls="--", lw=1, color="0.3")  # every sender busy all the time
            ax.annotate(
                f"M / μ = {cap:,.0f} msg/s", (1, cap), xycoords=("axes fraction", "data"), ha="right", va="bottom"
            )
        ax.margins(y=0.15)
        ax.yaxis.set_major_formatter(EngFormatter(sep=""))
        ax.spines[["top", "right"]].set_visible(False)
    pins = f"ports {d['commit'][:7]}, adi-sms {d['adi_sms'][:7]}"
    foot = f"{d['machine']['label']}; {pins}.\nmsg/s = N / the mean wall time of {RUNS} runs, start-up included; whiskers ± 1 sd."
    fig.supxlabel(foot, fontsize=8, x=0, ha="left")
    fig.savefig(ROOT / "bench/chart.svg", metadata={"Date": None, "Creator": None})


if __name__ == "__main__":
    match sys.argv[1:]:
        case ["xlang", label, *sub] if len(sub) < 2:
            bench(label, OUT.joinpath(*sub))
        case ["table"]:
            table()
        case ["chart"]:
            chart()
        case _:
            sys.exit(__doc__)
