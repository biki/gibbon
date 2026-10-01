import AppKit

// frame <in.png> <out.png> <out width> <dark: 0|1>
// Make a window capture look like a macOS window: round the corners, add a
// hairline edge and a soft shadow on a transparent margin, then scale the
// result to <out width> (see scripts/screenshots.sh).
let a = CommandLine.arguments
let img = NSImage(contentsOfFile: a[1])!
let rep0 = img.representations[0]
let w = CGFloat(rep0.pixelsWide), h = CGFloat(rep0.pixelsHigh)
let dark = a[4] == "1"
let radius: CGFloat = 30, pad: CGFloat = 90
let W = w + 2 * pad, H = h + 2 * pad
let outW = CGFloat(Double(a[3])!)
let scale = outW / W
let ow = Int((W * scale).rounded()), oh = Int((H * scale).rounded())
let rep = NSBitmapImageRep(
    bitmapDataPlanes: nil, pixelsWide: ow, pixelsHigh: oh, bitsPerSample: 8,
    samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
    bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
let g = NSGraphicsContext(bitmapImageRep: rep)!
g.imageInterpolation = .high
NSGraphicsContext.current = g
let ctx = g.cgContext
ctx.scaleBy(x: scale, y: scale)
let body = NSRect(x: pad, y: pad, width: w, height: h)
let shape = NSBezierPath(roundedRect: body, xRadius: radius, yRadius: radius)
// Shadow: a wide soft one and a tight one, as macOS draws.
for (blur, alpha, dy) in [(70.0, 0.40, -26.0), (8.0, 0.22, -2.0)] {
    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: 0, height: dy), blur: blur,
                  color: NSColor(white: 0, alpha: alpha).cgColor)
    NSColor.black.setFill()
    shape.fill()
    ctx.restoreGState()
}
ctx.saveGState()
shape.addClip()
img.draw(in: body, from: .zero, operation: .copy, fraction: 1)
ctx.restoreGState()
// Hairline edge: light inside a dark window, dark around a light one.
let edge = NSBezierPath(roundedRect: body.insetBy(dx: 1, dy: 1), xRadius: radius - 1, yRadius: radius - 1)
edge.lineWidth = 2
(dark ? NSColor(white: 1, alpha: 0.14) : NSColor(white: 0, alpha: 0.16)).setStroke()
edge.stroke()
NSGraphicsContext.restoreGraphicsState()
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: a[2]))
print(a[2], ow, oh)
