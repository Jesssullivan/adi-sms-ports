"""Public-safety gate, copied from adi-sms when its core dropped it: tracked text and commit messages name no
private infrastructure and carry no tool attribution. The denylist is stored hashed, so this gate never names
what it bans.
    python3 tools/public_safe.py              scan files and commit messages; exit 1 on any finding
    python3 tools/public_safe.py -            scan stdin, such as a pull request title
    python3 tools/public_safe.py --add WORD   add sha256("adi-sms:" + word) to tools/denylist.sha256
    python3 tools/public_safe.py --self-test  prove every pattern still fires"""

import hashlib
import re
import subprocess
import sys
from collections.abc import Iterator
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LIST = ROOT / "tools/denylist.sha256"
SKIP = {"tools/denylist.sha256", "tools/public_safe.py", "uv.lock", "flake.lock"}
SHAPES = {
    "ruling-id": re.compile(r"\bR-[A-Z]+\d*(?:-[A-Z0-9]+)*\b"),
    "cgnat-address": re.compile(r"\b100\.(?:6[4-9]|[7-9]\d|1[01]\d|12[0-7])\.\d{1,3}\.\d{1,3}\b"),
    "tailnet-dns": re.compile(r"\b[a-z0-9-]+\.ts\.net\b", re.I),
    "home-path": re.compile(r"(?:/home|/Users|/srv)/[A-Za-z][\w.-]*/"),
    "email": re.compile(
        r"\b[\w.+-]+@(?!users\.noreply\.github\.com|example\.(?:com|org))[\w-]+(?:\.[\w-]+)+\b"
    ),
    "ai-attribution": re.compile(
        "(?i)co-authored-by:.*\\b("
        + "cl"
        + "aude|anthropic|openai|copilot|gpt)\\b|generated with \\[?"
        + "cl"
        + "aude"
    ),
}
SIGNOFF = "Signed-off-by: Jess Sullivan <jess@" + "sulliwood.org>"  # allowed as a whole commit-message line only
TICKET = re.compile(r"\b([A-Z]{2,6})-\d+\b")  # tracker-style ids: the prefix is checked against the denylist
TOKEN = re.compile(r"[a-z0-9]+(?:[.-][a-z0-9]+)*")


def h(word: str) -> str:
    return hashlib.sha256(b"adi-sms:" + word.lower().encode()).hexdigest()


def findings(text: str, banned: set[str]) -> Iterator[tuple[int, str, str]]:
    for n, line in enumerate(text.splitlines(), 1):
        for name, rx in SHAPES.items():
            if m := rx.search(line):
                yield n, name, m.group(0)
        for m in TICKET.finditer(line):
            if h(m.group(1)) in banned:
                yield n, "tracker-id", m.group(0)
        for tok in TOKEN.findall(line.lower()):
            for w in {tok, *re.split(r"[.-]", tok)}:
                if h(w) in banned:
                    yield n, "denylisted", w


def message_findings(msg: str, banned: set[str]) -> Iterator[tuple[int, str, str]]:
    return findings("\n".join("" if line == SIGNOFF else line for line in msg.splitlines()), banned)


def scan(banned: set[str]) -> int:
    git = ["git", "-C", str(ROOT)]
    files = (
        subprocess.run(
            [*git, "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
            capture_output=True,
            check=True,
        )
        .stdout.decode()
        .split("\0")
    )
    bad = 0
    for f in filter(None, files):
        p = ROOT / f
        if f in SKIP or not p.is_file() or b"\0" in (raw := p.read_bytes())[:8192]:
            continue
        for n, kind, hit in findings(raw.decode("utf-8", "replace"), banned):
            print(f"{f}:{n}: {kind}: {hit}")
            bad += 1
    log = subprocess.run(
        [*git, "log", "--format=%h%x00%B%x01", "HEAD"], capture_output=True, text=True
    ).stdout
    for entry in filter(str.strip, log.split("\x01")):
        sha, _, msg = entry.strip().partition("\0")
        for _, kind, hit in message_findings(msg, banned):
            print(f"commit {sha}: {kind}: {hit}")
            bad += 1
    return bad


def self_test(banned: set[str]) -> None:
    probes = {
        "ruling-id": "R-" + "X12",
        "cgnat-address": "100." + "100.1.2",
        "tailnet-dns": "box.tail" + "x.ts.net",
        "home-path": "/ho" + "me/someone/",
        "email": "a" + "@b.io",
        "ai-attribution": "Co-Authored-By: " + "Cl" + "aude",
    }
    for kind, probe in probes.items():
        assert any(k == kind for _, k, _ in findings(probe, banned)), kind
    signed = "fix: a\n\n" + SIGNOFF
    assert not list(message_findings(signed, banned)), "sign-off line"
    assert list(findings(SIGNOFF, banned)), "sign-off outside a commit message"
    assert list(message_findings(signed + " ", banned)), "sign-off line not exact"
    assert list(message_findings(signed + "\nmail jess@" + "sulliwood.org", banned)), "email in another line"
    assert not list(findings("users at example.com: a@example.com, SHA-256, GSM-7", banned))


if __name__ == "__main__":
    banned = set(LIST.read_text().split()) if LIST.exists() else set()
    match sys.argv[1:]:
        case ["--add", word]:
            LIST.write_text("\n".join(sorted(banned | {h(word)})) + "\n")
        case ["-"]:
            hits = list(findings(sys.stdin.read(), banned))
            for line, kind, hit in hits:
                print(f"stdin:{line}: {kind}: {hit}")
            print(f"public-safe: {len(hits)} finding(s)" if hits else "public-safe: clean")
            sys.exit(1 if hits else 0)
        case ["--self-test"]:
            self_test(banned)
            print("public-safe: self-test ok")
        case []:
            self_test(banned)
            n = scan(banned)
            print(f"public-safe: {n} finding(s)" if n else "public-safe: clean")
            sys.exit(1 if n else 0)
        case _:
            sys.exit(__doc__)
