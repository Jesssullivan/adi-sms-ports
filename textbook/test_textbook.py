"""textbook/: MT19937 and TinyMT32 against the published reference outputs, the TS port against the Python, and the
textbook sim's totals. Run: python3 textbook/test_textbook.py (`just textbook`)."""

import json
import random
import shutil
import subprocess
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from mt import MT19937, TinyMT32  # noqa: E402
from sms import parse, pipeline  # noqa: E402

# init_genrand(5489), the default seed of mt19937ar.c and std::mt19937: the first five outputs; C++ [rand.predef] fixes the 10,000th at 4123659995
MT_FIRST = [3499211612, 581869302, 3890346734, 3586334585, 545404204]
# TinyMT tinymt/check32.out.txt, "tinymt32 0x8f7011ee 0xfc78ff1f 0x3793fdff seed = 1": all 50 32-bit outputs
TINY_SEED_1 = [
    2545341989, 981918433, 3715302833, 2387538352, 3591001365, 3820442102, 2114400566, 2196103051, 2783359912,
    764534509, 643179475, 1822416315, 881558334, 4207026366, 3690273640, 3240535687, 2921447122, 3984931427,
    4092394160, 44209675, 2188315343, 2908663843, 1834519336, 3774670961, 3019990707, 4065554902, 1239765502,
    4035716197, 3412127188, 552822483, 161364450, 353727785, 140085994, 149132008, 2547770827, 4064042525,
    4078297538, 2057335507, 622384752, 2041665899, 2193913817, 1080849512, 33160901, 662956935, 642999063,
    3384709977, 1723175122, 3866752252, 521822317, 2292524454,
]
TS_SEEDS = [0, 1, 42, 2**31, 2**32 - 1]


class Generators(unittest.TestCase):
    def test_mt19937_reference(self) -> None:
        mt = MT19937()
        out = [mt.genrand_int32() for _ in range(10_000)]
        self.assertEqual((out[:5], out[-1]), (MT_FIRST, 4123659995))

    def test_mt19937_matches_cpython(self) -> None:  # CPython's random is MT19937 in C; load our state into it
        mt, r = MT19937(2026), random.Random()
        r.setstate((3, (*mt.mt, 624), None))
        self.assertEqual([mt.genrand_int32() for _ in range(2000)], [r.getrandbits(32) for _ in range(2000)])

    def test_tinymt32_reference(self) -> None:
        t = TinyMT32(1)
        self.assertEqual([t.genrand_int32() for _ in range(50)], TINY_SEED_1)

    def test_floats_in_unit_interval(self) -> None:
        m, t = MT19937(7), TinyMT32(7)
        self.assertTrue(all(0 <= g.genrand_float() < 1 for g in (m, t) for _ in range(1000)))


class TypeScript(unittest.TestCase):
    def test_ts_matches_python(self) -> None:
        node = shutil.which("node")
        self.assertTrue(node, "node is in the devShell (nix develop)")
        js = (f"import {{ TinyMT32 }} from '{(HERE / 'tinymt.ts').as_uri()}';"
              f"console.log(JSON.stringify({TS_SEEDS}.map((s) => {{ const t = new TinyMT32(s);"
              "return [...Array.from({ length: 1000 }, () => t.genrandInt32()), t.genrandFloat()]; })));")
        out = subprocess.run([node, "--experimental-strip-types", "--no-warnings", "--input-type=module", "-e", js],
                             check=True, capture_output=True, text=True, timeout=60).stdout
        py = []
        for s in TS_SEEDS:
            t = TinyMT32(s)
            py.append([*(t.genrand_int32() for _ in range(1000)), t.genrand_float()])
        self.assertEqual(json.loads(out), py)

    def test_js_is_built_from_ts(self) -> None:  # rebuild with `just textbook-js`
        esbuild = shutil.which("esbuild")
        self.assertTrue(esbuild, "esbuild is in the devShell (nix develop)")
        built = subprocess.run([esbuild, str(HERE / "tinymt.ts"), "--format=iife", "--global-name=TinyMT",
                                "--log-level=warning"], check=True, capture_output=True, text=True, timeout=60).stdout
        self.assertEqual(built, (HERE / "tinymt.js").read_text())


class Sim(unittest.TestCase):
    def test_sim_totals(self) -> None:  # each of N messages is sent or failed once; p = 0 and p = 1 are exact
        n = 200
        for ppm, attempts in [(0, 3), (1_000_000, 3), (300_000, 3), (300_000, 1)]:
            with self.subTest(fail_ppm=ppm, attempts=attempts):
                a = parse(["--n", str(n), "--senders", "8", "--capacity", "4", "--mean-us", "500", "--backoff-us", "0",
                           "--fail-ppm", str(ppm), "--attempts", str(attempts), "--interval-ms", "60000"])
                sent, failed, tries, _ = (sum(col) for col in zip(*pipeline(a), strict=True))
                self.assertEqual(sent + failed, n)
                self.assertTrue(n <= tries <= attempts * n)
                if ppm == 0:
                    self.assertEqual((sent, tries), (n, n))
                if ppm == 1_000_000:
                    self.assertEqual((failed, tries), (n, attempts * n))


if __name__ == "__main__":
    unittest.main()
