// Draws the 1024×1024 app icon: a gold gibbon that hangs from a branch on a
// black rounded square. The colors are those of the Gibbon theme
// (src/theme.rs): siamang black, golden-cheeked gold and the cream face ring
// of a lar gibbon.
//   swiftc -O scripts/make-icon.swift -o target/icon/make-icon
//   target/icon/make-icon target/icon/icon-1024.png
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

let size = 1024
let out = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "icon-1024.png"
let space = CGColorSpace(name: CGColorSpace.sRGB)!
guard let ctx = CGContext(
    data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0,
    space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
) else { fatalError("no context") }

func rgb(_ hex: UInt32, _ a: CGFloat = 1) -> CGColor {
    CGColor(
        srgbRed: CGFloat((hex >> 16) & 0xFF) / 255,
        green: CGFloat((hex >> 8) & 0xFF) / 255,
        blue: CGFloat(hex & 0xFF) / 255, alpha: a)
}

// macOS icon grid: 824 px body with a 100 px margin, corner radius 185.
let body = CGRect(x: 100, y: 100, width: 824, height: 824)
let shape = CGPath(roundedRect: body, cornerWidth: 185, cornerHeight: 185, transform: nil)

// Soft shadow under the body.
ctx.saveGState()
ctx.setShadow(offset: CGSize(width: 0, height: -12), blur: 28, color: rgb(0x000000, 0.35))
ctx.addPath(shape)
ctx.setFillColor(rgb(0x171512))
ctx.fillPath()
ctx.restoreGState()

// Gradient body: warm gray at the top left, near black at the bottom right.
ctx.saveGState()
ctx.addPath(shape)
ctx.clip()
let gradient = CGGradient(
    colorsSpace: space, colors: [rgb(0x332E27), rgb(0x1E1B17), rgb(0x0F0E0C)] as CFArray,
    locations: [0, 0.55, 1])!
ctx.drawLinearGradient(
    gradient, start: CGPoint(x: 100, y: 924), end: CGPoint(x: 924, y: 100), options: [])
// A faint top highlight.
let shine = CGGradient(
    colorsSpace: space, colors: [rgb(0xFFFFFF, 0.10), rgb(0xFFFFFF, 0)] as CFArray,
    locations: [0, 1])!
ctx.drawLinearGradient(
    shine, start: CGPoint(x: 512, y: 924), end: CGPoint(x: 512, y: 560), options: [])
ctx.restoreGState()

// The gibbon (y grows upward in CoreGraphics). It hangs by one arm from a
// branch; its cream face ring doubles as a commit ring, and the free arm
// curves off like a git branch line.
let gold = rgb(0xF2AE3D)
let cream = rgb(0xF2EDE4)
let face = rgb(0x171512)
ctx.setStrokeColor(gold)
ctx.setFillColor(gold)
ctx.setLineCap(.round)
ctx.setLineJoin(.round)

func stroke(_ width: CGFloat, _ build: (CGMutablePath) -> Void) {
    let path = CGMutablePath()
    build(path)
    ctx.addPath(path)
    ctx.setLineWidth(width)
    ctx.strokePath()
}
func circle(_ x: CGFloat, _ y: CGFloat, _ r: CGFloat, _ color: CGColor) {
    ctx.setFillColor(color)
    ctx.fillEllipse(in: CGRect(x: x - r, y: y - r, width: 2 * r, height: 2 * r))
}

// The tree branch across the top, with one leaf.
stroke(36) { p in
    p.move(to: CGPoint(x: 180, y: 770))
    p.addCurve(to: CGPoint(x: 844, y: 792),
               control1: CGPoint(x: 400, y: 804), control2: CGPoint(x: 650, y: 756))
}
func leaf(_ x: CGFloat, _ y: CGFloat, _ angle: CGFloat, _ len: CGFloat) {
    ctx.saveGState()
    ctx.translateBy(x: x, y: y)
    ctx.rotate(by: angle)
    let p = CGMutablePath()
    p.move(to: .zero)
    p.addQuadCurve(to: CGPoint(x: len, y: 0), control: CGPoint(x: len / 2, y: len * 0.42))
    p.addQuadCurve(to: .zero, control: CGPoint(x: len / 2, y: -len * 0.42))
    ctx.addPath(p)
    ctx.setFillColor(gold)
    ctx.fillPath()
    ctx.restoreGState()
}
leaf(726, 790, 0.75, 96)
leaf(300, 792, 2.1, 70)

// The gibbon swings a little: turn it around the hand on the branch.
let hand = CGPoint(x: 612, y: 784)
ctx.saveGState()
ctx.translateBy(x: hand.x, y: hand.y)
ctx.rotate(by: 0.1)
ctx.translateBy(x: -hand.x, y: -hand.y)

// Raised arm: long, from the hand down to the shoulder.
stroke(36) { p in
    p.move(to: hand)
    p.addCurve(to: CGPoint(x: 548, y: 492),
               control1: CGPoint(x: 612, y: 690), control2: CGPoint(x: 560, y: 580))
}
circle(hand.x, hand.y, 28, gold)

// Body, small and hanging below the head.
ctx.saveGState()
ctx.translateBy(x: 505, y: 334)
ctx.rotate(by: -0.08)
ctx.setFillColor(gold)
ctx.addPath(CGPath(roundedRect: CGRect(x: -60, y: -100, width: 120, height: 200),
                   cornerWidth: 60, cornerHeight: 60, transform: nil))
ctx.fillPath()
ctx.restoreGState()

// Legs, tucked up.
stroke(34) { p in
    p.move(to: CGPoint(x: 476, y: 252))
    p.addCurve(to: CGPoint(x: 408, y: 236),
               control1: CGPoint(x: 460, y: 206), control2: CGPoint(x: 422, y: 206))
}
stroke(34) { p in
    p.move(to: CGPoint(x: 540, y: 246))
    p.addCurve(to: CGPoint(x: 612, y: 222),
               control1: CGPoint(x: 560, y: 200), control2: CGPoint(x: 598, y: 196))
}

// Free arm: stretched out wide to the left and up, like a branch line
// that ends in a commit dot.
stroke(36) { p in
    p.move(to: CGPoint(x: 462, y: 404))
    p.addCurve(to: CGPoint(x: 214, y: 486),
               control1: CGPoint(x: 370, y: 350), control2: CGPoint(x: 240, y: 380))
}
circle(214, 500, 30, gold)

// Head: cream fur ring around a dark face, like a ring commit dot.
circle(500, 470, 86, cream)
circle(500, 466, 56, face)
circle(481, 476, 9, cream)
circle(519, 476, 9, cream)
ctx.restoreGState()

guard let image = ctx.makeImage(),
      let dest = CGImageDestinationCreateWithURL(
          URL(fileURLWithPath: out) as CFURL, UTType.png.identifier as CFString, 1, nil)
else { fatalError("cannot write \(out)") }
CGImageDestinationAddImage(dest, image, nil)
guard CGImageDestinationFinalize(dest) else { fatalError("cannot write \(out)") }
print(out)
