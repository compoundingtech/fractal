// Green is hue 90-170° with HSL saturation above 25%; any colour function the parser cannot read fails too.
export function greenFindings(root: Element): string[] {
  const findings: string[] = []
  const properties = ['color', 'background-color', 'border-top-color', 'border-right-color', 'border-bottom-color', 'border-left-color', 'outline-color', 'text-decoration-color', 'caret-color', 'fill', 'stroke', 'box-shadow', 'background-image']
  for (const element of [root, ...root.querySelectorAll('*')]) for (const pseudo of [null, '::before', '::after']) {
    const style = getComputedStyle(element, pseudo)
    for (const property of properties) {
      const value = style.getPropertyValue(property)
      if (/\b(?:oklch|oklab|lab|lch|hsla?|hwb)\(/.test(value)) { findings.push(`${property} unparsed ${value}`); continue }
      for (const match of value.matchAll(/rgba?\(([^)]*)\)|color\(srgb ([^)]*)\)/g)) {
        const parts = (match[1] ?? match[2]!).split(/[\s,/]+/).filter(Boolean).map(Number)
        const scale = match[1] === undefined ? 1 : 255
        const [red, green, blue] = parts.slice(0, 3).map(part => part / scale) as [number, number, number]
        if ((parts[3] ?? 1) === 0) continue
        const max = Math.max(red, green, blue), min = Math.min(red, green, blue), lightness = (max + min) / 2
        if (max === min) continue
        const saturation = (max - min) / (1 - Math.abs(2 * lightness - 1))
        const hue = (max === red ? ((green - blue) / (max - min) + 6) % 6 : max === green ? (blue - red) / (max - min) + 2 : (red - green) / (max - min) + 4) * 60
        if (hue >= 90 && hue <= 170 && saturation > 0.25) findings.push(`${element.tagName.toLowerCase()}${pseudo ?? ''}[${element.getAttribute('data-syntax-token') ?? element.getAttribute('data-testid') ?? ''}] ${property} ${match[0]}`)
      }
    }
  }
  return findings
}
