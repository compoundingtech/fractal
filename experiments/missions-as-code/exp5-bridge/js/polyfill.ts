// Pure-JS stand-ins for the few globals Effect touches at load. None of them reach the host.
const g = globalThis as any
if (typeof g.TextEncoder === "undefined") {
  g.TextEncoder = class {
    readonly encoding = "utf-8"
    encode(text = ""): Uint8Array {
      const bytes: Array<number> = []
      for (const ch of text) {
        let c = ch.codePointAt(0)!
        if (c < 0x80) bytes.push(c)
        else if (c < 0x800) bytes.push(0xc0 | (c >> 6), 0x80 | (c & 63))
        else if (c < 0x10000) bytes.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63))
        else bytes.push(0xf0 | (c >> 18), 0x80 | ((c >> 12) & 63), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63))
      }
      return new Uint8Array(bytes)
    }
  }
}
if (typeof g.TextDecoder === "undefined") {
  g.TextDecoder = class {
    readonly encoding = "utf-8"
    decode(bytes?: Uint8Array): string {
      let out = ""
      if (!bytes) return out
      for (let i = 0; i < bytes.length; ) {
        const b = bytes[i++]!
        let c = b
        if (b >= 0xf0) c = ((b & 7) << 18) | ((bytes[i++]! & 63) << 12) | ((bytes[i++]! & 63) << 6) | (bytes[i++]! & 63)
        else if (b >= 0xe0) c = ((b & 15) << 12) | ((bytes[i++]! & 63) << 6) | (bytes[i++]! & 63)
        else if (b >= 0xc0) c = ((b & 31) << 6) | (bytes[i++]! & 63)
        out += String.fromCodePoint(c)
      }
      return out
    }
  }
}
export {}
