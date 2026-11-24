#!/usr/bin/env swift
// Regenerates the committed brand assets from their SVG sources:
//   Resources/AppIcon/fractal-app-macos{,-32,-16}.svg -> Resources/AppIcon.icns
//   Resources/MenuBar/fractal-menubar-*.svg           -> Sources/FractalMacUI/Resources/MenuBar/*.pdf
// Run from anywhere: `swift apps/macos/Fractal/scripts/generate-icons.swift`.
//
// The 16 and 32 px icon slots use their own small-size masters (same artwork, heavier mark
// stroke) so the mark stays legible at list sizes. Menu-bar PDFs are written directly (vector
// paths, even-odd clip for cut-outs, no images, no timestamps), so the output is byte-for-byte
// reproducible and only ever alpha for template recoloring.

import AppKit
import Foundation
import ImageIO
import UniformTypeIdentifiers

let appRoot = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
let iconSources = appRoot.appendingPathComponent("Resources/AppIcon")
let iconOutput = appRoot.appendingPathComponent("Resources/AppIcon.icns")
let glyphSources = appRoot.appendingPathComponent("Resources/MenuBar")
let glyphOutput = appRoot.appendingPathComponent("Sources/FractalMacUI/Resources/MenuBar")

func fail(_ message: String) -> Never {
  FileHandle.standardError.write(Data("generate-icons: \(message)\n".utf8))
  exit(1)
}

// MARK: - App icon

/// The master each rendered pixel size uses: 16 px and 32 px have heavier small-size masters.
func iconSVG(pixels: Int) -> Data {
  let name =
    switch pixels {
    case 16: "fractal-app-macos-16.svg"
    case 32: "fractal-app-macos-32.svg"
    default: "fractal-app-macos.svg"
    }
  guard let svg = try? Data(contentsOf: iconSources.appendingPathComponent(name)) else { fail("cannot read \(name)") }
  return svg
}

/// The tile's `feDropShadow`. NSImage's SVG decoder ignores filters, so the generator removes the
/// filter reference and draws the same shadow with Core Graphics (blur radius = 2 σ).
struct DropShadow {
  let dx: Double, dy: Double, deviation: Double, opacity: Double

  /// Splits the master into filter-free SVG data and the one drop shadow it declares.
  static func extract(from svg: Data) -> (svg: Data, shadow: DropShadow) {
    let text = String(decoding: svg, as: UTF8.self)
    let pattern = try! NSRegularExpression(
      pattern: ##"<feDropShadow dx="([-\d.]+)" dy="([-\d.]+)" stdDeviation="([\d.]+)" flood-color="#000" flood-opacity="([\d.]+)"/>"##)
    let matches = pattern.matches(in: text, range: NSRange(text.startIndex..., in: text))
    let reference = " filter=\"url(#shadow)\""
    guard matches.count == 1, text.components(separatedBy: reference).count == 2 else {
      fail("icon masters must declare exactly one black #shadow feDropShadow used once")
    }
    let value = { (index: Int) in Double(text[Range(matches[0].range(at: index), in: text)!])! }
    let shadow = DropShadow(dx: value(1), dy: value(2), deviation: value(3), opacity: value(4))
    return (Data(text.replacingOccurrences(of: reference, with: "").utf8), shadow)
  }
}

func png(svg master: Data, pixels: Int) -> Data {
  let (svg, shadow) = DropShadow.extract(from: master)
  guard let image = NSImage(data: svg) else { fail("NSImage cannot decode the icon SVG") }
  guard
    let context = CGContext(
      data: nil, width: pixels, height: pixels, bitsPerComponent: 8, bytesPerRow: 0,
      space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
  else { fail("cannot create a \(pixels) px bitmap") }
  // Shadow geometry is in device pixels and y-up; the masters are a 1024 px y-down canvas.
  let scale = Double(pixels) / image.size.width
  context.setShadow(
    offset: CGSize(width: shadow.dx * scale, height: -shadow.dy * scale), blur: 2 * shadow.deviation * scale,
    color: CGColor(gray: 0, alpha: shadow.opacity))
  context.beginTransparencyLayer(auxiliaryInfo: nil)
  NSGraphicsContext.saveGraphicsState()
  NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: false)
  image.draw(in: NSRect(x: 0, y: 0, width: pixels, height: pixels))
  NSGraphicsContext.restoreGraphicsState()
  context.endTransparencyLayer()
  let data = NSMutableData()
  guard
    let cgImage = context.makeImage(),
    let destination = CGImageDestinationCreateWithData(data, UTType.png.identifier as CFString, 1, nil)
  else { fail("cannot encode a \(pixels) px PNG") }
  CGImageDestinationAddImage(destination, cgImage, nil)
  guard CGImageDestinationFinalize(destination) else { fail("cannot encode a \(pixels) px PNG") }
  return data as Data
}

func writeAppIcon() {
  let fileManager = FileManager.default
  let iconset = fileManager.temporaryDirectory.appendingPathComponent("FractalAppIcon-\(UUID().uuidString).iconset")
  try! fileManager.createDirectory(at: iconset, withIntermediateDirectories: true)
  defer { try? fileManager.removeItem(at: iconset) }
  for points in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
      let pixels = points * scale
      let name = scale == 1 ? "icon_\(points)x\(points).png" : "icon_\(points)x\(points)@2x.png"
      try! png(svg: iconSVG(pixels: pixels), pixels: pixels).write(to: iconset.appendingPathComponent(name))
    }
  }
  let iconutil = Process()
  iconutil.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
  iconutil.arguments = ["-c", "icns", iconset.path, "-o", iconOutput.path]
  try! iconutil.run()
  iconutil.waitUntilExit()
  guard iconutil.terminationStatus == 0 else { fail("iconutil failed") }
  print("wrote \(iconOutput.path)")
}

// MARK: - Menu-bar glyphs

/// A 2D affine transform in PDF `cm` order.
struct Affine {
  var a = 1.0, b = 0.0, c = 0.0, d = 1.0, e = 0.0, f = 0.0
  var isIdentity: Bool { a == 1 && b == 0 && c == 0 && d == 1 && e == 0 && f == 0 }

  /// Supports the `translate(x,y)` and `scale(s)` sequence the glyph sources use.
  init(svg: String?) {
    guard let svg else { return }
    let pattern = try! NSRegularExpression(pattern: #"(translate|scale)\(([^)]*)\)"#)
    let range = NSRange(svg.startIndex..., in: svg)
    for match in pattern.matches(in: svg, range: range) {
      let name = String(svg[Range(match.range(at: 1), in: svg)!])
      let values = svg[Range(match.range(at: 2), in: svg)!]
        .split(whereSeparator: { $0 == "," || $0 == " " }).map { Double($0)! }
      if name == "translate" {
        e += a * values[0] + c * values[1]
        f += b * values[0] + d * values[1]
      } else {
        let sx = values[0], sy = values.count > 1 ? values[1] : values[0]
        a *= sx; b *= sx; c *= sy; d *= sy
      }
    }
    let rest = pattern.stringByReplacingMatches(in: svg, range: range, withTemplate: "")
    guard rest.trimmingCharacters(in: .whitespaces).isEmpty else { fail("unsupported transform \(svg)") }
  }

  init() {}
}

func number(_ value: Double) -> String {
  var text = String(format: "%.5f", value)
  while text.hasSuffix("0") { text.removeLast() }
  if text.hasSuffix(".") { text.removeLast() }
  return text == "-0" ? "0" : text
}

func attribute(_ attributes: [String: String], _ name: String) -> Double {
  guard let text = attributes[name], let value = Double(text) else { fail("missing numeric \(name)") }
  return value
}

/// Path construction operators for the shapes the glyph sources use.
enum Shape {
  static let kappa = 0.5522847498307936

  static func polygon(_ attributes: [String: String]) -> String {
    guard let points = attributes["points"] else { fail("polygon without points") }
    let pairs = points.split(separator: " ").map { $0.split(separator: ",").map { Double($0)! } }
    var path = "\(number(pairs[0][0])) \(number(pairs[0][1])) m\n"
    for pair in pairs.dropFirst() { path += "\(number(pair[0])) \(number(pair[1])) l\n" }
    return path + "h\n"
  }

  static func circle(_ attributes: [String: String]) -> String {
    ellipse(
      cx: attribute(attributes, "cx"), cy: attribute(attributes, "cy"),
      rx: attribute(attributes, "r"), ry: attribute(attributes, "r"))
  }

  static func ellipse(cx: Double, cy: Double, rx: Double, ry: Double) -> String {
    let kx = rx * kappa, ky = ry * kappa
    return """
      \(number(cx + rx)) \(number(cy)) m
      \(number(cx + rx)) \(number(cy + ky)) \(number(cx + kx)) \(number(cy + ry)) \(number(cx)) \(number(cy + ry)) c
      \(number(cx - kx)) \(number(cy + ry)) \(number(cx - rx)) \(number(cy + ky)) \(number(cx - rx)) \(number(cy)) c
      \(number(cx - rx)) \(number(cy - ky)) \(number(cx - kx)) \(number(cy - ry)) \(number(cx)) \(number(cy - ry)) c
      \(number(cx + kx)) \(number(cy - ry)) \(number(cx + rx)) \(number(cy - ky)) \(number(cx + rx)) \(number(cy)) c
      h

      """
  }

  static func rect(_ attributes: [String: String]) -> String {
    let x = attribute(attributes, "x"), y = attribute(attributes, "y")
    let width = attribute(attributes, "width"), height = attribute(attributes, "height")
    let rx = min(attributes["rx"].flatMap(Double.init) ?? attributes["ry"].flatMap(Double.init) ?? 0, width / 2)
    let ry = min(attributes["ry"].flatMap(Double.init) ?? rx, height / 2)
    if rx == 0 || ry == 0 { return "\(number(x)) \(number(y)) \(number(width)) \(number(height)) re\n" }
    let kx = rx * kappa, ky = ry * kappa
    let right = x + width, bottom = y + height
    return """
      \(number(x + rx)) \(number(y)) m
      \(number(right - rx)) \(number(y)) l
      \(number(right - rx + kx)) \(number(y)) \(number(right)) \(number(y + ry - ky)) \(number(right)) \(number(y + ry)) c
      \(number(right)) \(number(bottom - ry)) l
      \(number(right)) \(number(bottom - ry + ky)) \(number(right - rx + kx)) \(number(bottom)) \(number(right - rx)) \(number(bottom)) c
      \(number(x + rx)) \(number(bottom)) l
      \(number(x + rx - kx)) \(number(bottom)) \(number(x)) \(number(bottom - ry + ky)) \(number(x)) \(number(bottom - ry)) c
      \(number(x)) \(number(y + ry)) l
      \(number(x)) \(number(y + ry - ky)) \(number(x + rx - kx)) \(number(y)) \(number(x + rx)) \(number(y)) c
      h

      """
  }
}

/// Translates one glyph SVG into a PDF content stream. Only the subset the glyph sources use is
/// accepted (groups with translate/scale, polygon, circle, rect, black paint, and luminance masks
/// made of one white rect with black cut-outs, on a group or a shape); anything else stops
/// generation instead of being approximated. A mask becomes an even-odd clip, so cut-outs are
/// true transparency.
final class GlyphTranslator: NSObject, XMLParserDelegate {
  var width = 0.0, height = 0.0
  var content = ""
  private var masks: [String: String] = [:]
  private var currentMask: (id: String, area: String?, holes: String)?

  func parser(
    _ parser: XMLParser, didStartElement element: String, namespaceURI: String?, qualifiedName: String?,
    attributes: [String: String] = [:]
  ) {
    if let opacity = attributes["opacity"], Double(opacity) != 1 { fail("unsupported opacity on <\(element)>") }
    if var mask = currentMask {
      let path: String
      switch element {
      case "rect": path = Shape.rect(attributes)
      case "circle": path = Shape.circle(attributes)
      default: fail("unsupported <\(element)> in mask \(mask.id)")
      }
      switch attributes["fill"] {
      case "white" where mask.area == nil: mask.area = path
      case "black": mask.holes += path
      default: fail("mask \(mask.id) must be one white area with black cut-outs")
      }
      currentMask = mask
      return
    }
    switch element {
    case "svg":
      let artWidth = attribute(attributes, "width"), artHeight = attribute(attributes, "height")
      guard attributes["viewBox"] == "0 0 \(attributes["width"]!) \(attributes["height"]!)" else {
        fail("viewBox must match width/height")
      }
      // NSPDFImageRep rounds a fractional MediaBox up and draws from the origin, which would
      // push the glyph off-center; pad the page to whole points and center the artwork instead.
      width = artWidth.rounded(.up)
      height = artHeight.rounded(.up)
      // PDF space is y-up; SVG space is y-down.
      content += "1 0 0 -1 \(number((width - artWidth) / 2)) \(number(height - (height - artHeight) / 2)) cm\n"
    case "g":
      content += "q\n"
      let transform = Affine(svg: attributes["transform"])
      if !transform.isIdentity {
        content += [transform.a, transform.b, transform.c, transform.d, transform.e, transform.f]
          .map(number).joined(separator: " ") + " cm\n"
      }
      if let reference = attributes["mask"] { content += clip(reference) }
    case "defs": break
    case "mask":
      guard attributes["maskUnits"] == "userSpaceOnUse", let id = attributes["id"] else {
        fail("masks must use userSpaceOnUse and have an id")
      }
      currentMask = (id, nil, "")
    case "polygon": paint(Shape.polygon(attributes), attributes)
    case "circle": paint(Shape.circle(attributes), attributes)
    case "rect": paint(Shape.rect(attributes), attributes)
    default: fail("unsupported <\(element)>")
    }
  }

  func parser(
    _ parser: XMLParser, didEndElement element: String, namespaceURI: String?, qualifiedName: String?
  ) {
    switch element {
    case "mask":
      guard let mask = currentMask, let area = mask.area, !mask.holes.isEmpty else { fail("empty mask") }
      masks[mask.id] = area + mask.holes
      currentMask = nil
    case "g":
      content += "Q\n"
    default: break
    }
  }

  /// The even-odd clip for a `url(#id)` mask reference.
  private func clip(_ reference: String) -> String {
    let id = reference.replacingOccurrences(of: "url(#", with: "").replacingOccurrences(of: ")", with: "")
    guard let clip = masks[id] else { fail("mask \(id) used before definition") }
    return clip + "W* n\n"
  }

  private func paint(_ path: String, _ attributes: [String: String]) {
    let fill = attributes["fill"] ?? "#000"
    let stroke = attributes["stroke"] ?? "none"
    for paint in [fill, stroke] where paint != "none" && paint != "#000" {
      fail("glyphs are templates: paint must be #000 or none, got \(paint)")
    }
    var operators = ""
    if stroke != "none" {
      // SVG defaults are butt caps and miter joins.
      let cap =
        switch attributes["stroke-linecap"] ?? "butt" {
        case "butt": 0
        case "round": 1
        case let other: fail("unsupported stroke-linecap \(other)")
        }
      let join =
        switch attributes["stroke-linejoin"] ?? "miter" {
        case "miter": 0
        case "round": 1
        case let other: fail("unsupported stroke-linejoin \(other)")
        }
      operators += "\(number(attribute(attributes, "stroke-width"))) w \(cap) J \(join) j\n"
    }
    let op =
      switch (fill != "none", stroke != "none") {
      case (true, true): "B"
      case (true, false): "f"
      case (false, true): "S"
      case (false, false): fail("shape paints nothing")
      }
    if let reference = attributes["mask"] {
      content += "q\n" + clip(reference) + operators + path + op + "\nQ\n"
    } else {
      content += operators + path + op + "\n"
    }
  }
}

/// A single-page PDF with no metadata, so regeneration is byte-identical.
func pdf(width: Double, height: Double, content: String) -> Data {
  let objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 \(number(width)) \(number(height))] /Resources << >> /Contents 4 0 R >>",
    "<< /Length \(content.utf8.count) >>\nstream\n\(content)endstream",
  ]
  var output = "%PDF-1.4\n"
  var offsets: [Int] = []
  for (index, object) in objects.enumerated() {
    offsets.append(output.utf8.count)
    output += "\(index + 1) 0 obj\n\(object)\nendobj\n"
  }
  let xref = output.utf8.count
  output += "xref\n0 \(objects.count + 1)\n0000000000 65535 f \n"
  for offset in offsets { output += String(format: "%010d 00000 n \n", offset) }
  output += "trailer\n<< /Size \(objects.count + 1) /Root 1 0 R >>\nstartxref\n\(xref)\n%%EOF\n"
  return Data(output.utf8)
}

func writeMenuBarGlyphs() {
  let fileManager = FileManager.default
  try! fileManager.createDirectory(at: glyphOutput, withIntermediateDirectories: true)
  let sources = try! fileManager.contentsOfDirectory(at: glyphSources, includingPropertiesForKeys: nil)
    .filter { $0.pathExtension == "svg" }.sorted { $0.lastPathComponent < $1.lastPathComponent }
  guard !sources.isEmpty else { fail("no glyph SVGs in \(glyphSources.path)") }
  for source in sources {
    let translator = GlyphTranslator()
    let parser = XMLParser(contentsOf: source)!
    parser.delegate = translator
    guard parser.parse() else { fail("cannot parse \(source.lastPathComponent)") }
    let output = glyphOutput.appendingPathComponent(source.deletingPathExtension().lastPathComponent + ".pdf")
    try! pdf(width: translator.width, height: translator.height, content: translator.content).write(to: output)
    print("wrote \(output.path)")
  }
}

writeAppIcon()
writeMenuBarGlyphs()
