var TinyMT = (() => {
  var __defProp = Object.defineProperty;
  var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
  var __getOwnPropNames = Object.getOwnPropertyNames;
  var __hasOwnProp = Object.prototype.hasOwnProperty;
  var __export = (target, all) => {
    for (var name in all)
      __defProp(target, name, { get: all[name], enumerable: true });
  };
  var __copyProps = (to, from, except, desc) => {
    if (from && typeof from === "object" || typeof from === "function") {
      for (let key of __getOwnPropNames(from))
        if (!__hasOwnProp.call(to, key) && key !== except)
          __defProp(to, key, { get: () => from[key], enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable });
    }
    return to;
  };
  var __toCommonJS = (mod) => __copyProps(__defProp({}, "__esModule", { value: true }), mod);
  var tinymt_exports = {};
  __export(tinymt_exports, {
    TinyMT32: () => TinyMT32
  });
  const MAT1 = 2406486510;
  const MAT2 = 4235788063;
  const TMAT = 932445695;
  class TinyMT32 {
    s;
    constructor(seed) {
      const s = new Uint32Array([seed, MAT1, MAT2, TMAT]);
      for (let i = 1; i < 8; i++) {
        const p = s[i - 1 & 3];
        s[i & 3] ^= i + Math.imul(1812433253, p ^ p >>> 30);
      }
      if ((s[0] & 2147483647) === 0 && s[1] === 0 && s[2] === 0 && s[3] === 0) s.set([84, 73, 78, 89]);
      this.s = s;
      for (let i = 0; i < 8; i++) this.nextState();
    }
    nextState() {
      const s = this.s;
      let x = s[0] & 2147483647 ^ s[1] ^ s[2];
      x ^= x << 1;
      const y = s[3] ^ s[3] >>> 1 ^ x;
      const mask = -(y & 1);
      s[0] = s[1];
      s[1] = s[2] ^ mask & MAT1;
      s[2] = x ^ y << 10 ^ mask & MAT2;
      s[3] = y;
    }
    genrandInt32() {
      this.nextState();
      const t1 = this.s[0] + (this.s[2] >>> 8) >>> 0;
      return (this.s[3] ^ t1 ^ -(t1 & 1) & TMAT) >>> 0;
    }
    genrandFloat() {
      return (this.genrandInt32() >>> 8) / 16777216;
    }
  }
  return __toCommonJS(tinymt_exports);
})();
