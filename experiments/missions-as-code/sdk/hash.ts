// A small deterministic fingerprint for op specs. It needs no crypto module, so it runs the same
// in Node, QuickJS, Boa and V8. It detects a changed definition; it is not a security boundary.

export const canonicalJson = (value: unknown): string => {
  if (value === null || typeof value !== "object") return JSON.stringify(value ?? null)
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`
  const keys = Object.keys(value as object).filter((k) => (value as any)[k] !== undefined).sort()
  return `{${keys.map((k) => `${JSON.stringify(k)}:${canonicalJson((value as any)[k])}`).join(",")}}`
}

// Two independent FNV-1a 32-bit passes give a 64-bit fingerprint.
export const hashJson = (value: unknown): string => {
  const text = typeof value === "string" ? value : canonicalJson(value)
  let a = 0x811c9dc5
  let b = 0x01000193 ^ 0x5bd1e995
  for (let i = 0; i < text.length; i++) {
    const c = text.charCodeAt(i)
    a = Math.imul(a ^ c, 0x01000193) >>> 0
    b = Math.imul(b ^ c, 0x5bd1e995) >>> 0
    b ^= b >>> 13
  }
  return a.toString(16).padStart(8, "0") + b.toString(16).padStart(8, "0")
}
