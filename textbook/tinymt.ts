// TinyMT32 (Saito and Matsumoto 2011) as in mt.py: same seed, same outputs. 32-bit math without BigInt:
// Math.imul multiplies mod 2^32, `>>> 0` reads a result as unsigned, and a Uint32Array stores mod 2^32.
const MAT1 = 0x8f7011ee, MAT2 = 0xfc78ff1f, TMAT = 0x3793fdff;

export class TinyMT32 {
  s: Uint32Array;

  constructor(seed: number) {
    const s = new Uint32Array([seed, MAT1, MAT2, TMAT]);
    for (let i = 1; i < 8; i++) {
      const p = s[(i - 1) & 3];
      s[i & 3] ^= i + Math.imul(1812433253, p ^ (p >>> 30));
    }
    if ((s[0] & 0x7fffffff) === 0 && s[1] === 0 && s[2] === 0 && s[3] === 0) s.set([84, 73, 78, 89]); // "TINY"
    this.s = s;
    for (let i = 0; i < 8; i++) this.nextState();
  }

  nextState(): void {
    const s = this.s;
    let x = (s[0] & 0x7fffffff) ^ s[1] ^ s[2];
    x ^= x << 1;
    const y = s[3] ^ (s[3] >>> 1) ^ x;
    const mask = -(y & 1); // 0, or all ones when y is odd
    s[0] = s[1];
    s[1] = s[2] ^ (mask & MAT1);
    s[2] = x ^ (y << 10) ^ (mask & MAT2);
    s[3] = y;
  }

  genrandInt32(): number {
    this.nextState();
    const t1 = (this.s[0] + (this.s[2] >>> 8)) >>> 0; // tempering
    return (this.s[3] ^ t1 ^ (-(t1 & 1) & TMAT)) >>> 0;
  }

  genrandFloat(): number { // [0, 1) in steps of 2^-24
    return (this.genrandInt32() >>> 8) / 16777216;
  }
}
