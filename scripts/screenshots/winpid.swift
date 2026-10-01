// winpid <pid>: print the number of the largest on-screen window of a
// process, for `screencapture -l` (see scripts/screenshots.sh).
import CoreGraphics
import Foundation
let pid = Int32(CommandLine.arguments[1])!
let list = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID) as! [[String: Any]]
var best: (Int, Double) = (0, 0)
for w in list where (w[kCGWindowOwnerPID as String] as? Int32) == pid {
    let b = w[kCGWindowBounds as String] as! [String: Double]
    let area = b["Width"]! * b["Height"]!
    if area > best.1 { best = (w[kCGWindowNumber as String] as! Int, area) }
}
if best.0 != 0 { print(best.0) }
