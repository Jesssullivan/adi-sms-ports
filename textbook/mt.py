"""Two stateful PRNGs from Hiroshima University, written out from the reference C.

MT19937: Matsumoto and Nishimura (1998), mt19937ar.c. 624 words of state, period 2^19937 - 1.
TinyMT32: Saito and Matsumoto (2011), tinymt32.c with the default parameters. 4 words, period 2^127 - 1.
Each call advances the state, so a draw depends on every draw before it on the same generator.
"""

M32 = 0xFFFFFFFF


class MT19937:
    def __init__(self, seed: int = 5489) -> None:  # init_genrand
        mt = [seed & M32]
        for i in range(1, 624):
            mt.append((1812433253 * (mt[-1] ^ (mt[-1] >> 30)) + i) & M32)
        self.mt, self.i = mt, 624

    def twist(self) -> None:  # all 624 words at once; in place, so words past k + 397 >= 624 are new ones
        mt = self.mt
        for k in range(624):
            y = (mt[k] & 0x80000000) | (mt[(k + 1) % 624] & 0x7FFFFFFF)
            mt[k] = mt[(k + 397) % 624] ^ (y >> 1) ^ (0x9908B0DF if y & 1 else 0)
        self.i = 0

    def genrand_int32(self) -> int:
        if self.i == 624:
            self.twist()
        y = self.mt[self.i]
        self.i += 1
        y ^= y >> 11  # tempering: spreads the state bits into the output
        y ^= (y << 7) & 0x9D2C5680
        y ^= (y << 15) & 0xEFC60000
        return y ^ (y >> 18)

    def genrand_float(self) -> float:  # genrand_real2: [0, 1) in steps of 2^-32
        return self.genrand_int32() * 2.0**-32


class TinyMT32:
    MAT1, MAT2, TMAT = 0x8F7011EE, 0xFC78FF1F, 0x3793FDFF

    def __init__(self, seed: int) -> None:  # tinymt32_init
        s = [seed & M32, self.MAT1, self.MAT2, self.TMAT]
        for i in range(1, 8):
            s[i & 3] ^= (i + 1812433253 * (s[(i - 1) & 3] ^ (s[(i - 1) & 3] >> 30))) & M32
        if s[0] & 0x7FFFFFFF == 0 and s[1] == s[2] == s[3] == 0:  # period certification: never all zero
            s = [ord(c) for c in "TINY"]
        self.s = s
        for _ in range(8):
            self.next_state()

    def next_state(self) -> None:
        s0, s1, s2, s3 = self.s
        x = (s0 & 0x7FFFFFFF) ^ s1 ^ s2
        x ^= (x << 1) & M32
        y = s3 ^ (s3 >> 1) ^ x
        mat1, mat2 = (self.MAT1, self.MAT2) if y & 1 else (0, 0)
        self.s = [s1, s2 ^ mat1, ((x ^ (y << 10)) & M32) ^ mat2, y]

    def genrand_int32(self) -> int:
        self.next_state()
        s0, _, s2, s3 = self.s
        t1 = (s0 + (s2 >> 8)) & M32  # tempering
        return s3 ^ t1 ^ (self.TMAT if t1 & 1 else 0)

    def genrand_float(self) -> float:  # tinymt32_generate_float: [0, 1) in steps of 2^-24
        return (self.genrand_int32() >> 8) * 2.0**-24
