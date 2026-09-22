// Renders the tray (menu-bar / notification-area) status icons into
// src-tauri/icons/tray/. Run on a Mac:
//
//   swift scripts/make_tray_icons.swift
//
// macOS  (mac-<state>.png, 44 px = 22 pt @2x) from SF Symbols. Monochrome icons
//        are drawn black and set as template images, so macOS tints them for
//        light/dark menu bars; coloured ones keep their colour.
// Windows/Linux (win-<state>-dark.png / -light.png / .png, 32 px) from Google
//        Material Symbols Rounded (Apache-2.0), fetched from
//        github.com/google/material-design-icons. SF Symbols may only ship on
//        Apple platforms. Monochrome icons get a white version for dark taskbars
//        (`-dark`) and a black one for light taskbars (`-light`).

import AppKit

let root = URL(fileURLWithPath: CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "src-tauri/icons/tray")
try? FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)

let red = NSColor(srgbRed: 1.0, green: 0.23, blue: 0.19, alpha: 1)
let orange = NSColor(srgbRed: 1.0, green: 0.58, blue: 0.0, alpha: 1)
let green = NSColor(srgbRed: 0.19, green: 0.82, blue: 0.35, alpha: 1)

// state id, SF Symbol, Material Symbol file stem, colour (nil = monochrome)
let states: [(String, String, String, NSColor?)] = [
    ("ready", "mic", "mic", nil),
    ("model-unloaded", "moon.zzz", "bedtime", nil),
    ("no-model", "arrow.down.circle", "download", nil),
    ("downloading", "arrow.down.circle.dotted", "downloading", nil),
    ("loading-model", "memorychip", "memory", nil),
    ("dictating", "mic.fill", "mic_fill1", red),
    ("paused", "pause.circle.fill", "pause_circle_fill1", orange),
    ("processing-speech", "waveform", "graphic_eq", nil),
    ("grammar", "wand.and.stars", "auto_fix_high", nil),
    ("done", "checkmark.circle.fill", "check_circle_fill1", green),
    ("nothing-heard", "ear.trianglebadge.exclamationmark", "hearing_disabled", orange),
    ("paste-failed", "doc.on.clipboard", "content_paste_off", orange),
    ("error", "xmark.octagon.fill", "error_fill1", red),
    ("mic-blocked", "mic.slash.fill", "mic_off_fill1", red),
    ("processing-meeting", "person.2.wave.2.fill", "groups_fill1", nil),
    ("processing-file", "doc.text.fill", "description_fill1", nil),
    ("cancelled", "xmark.circle", "cancel", nil),
    ("call", "video.fill", "videocam_fill1", nil),
]

func canvas(_ px: Int, _ draw: () -> Void) -> Data {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px, bitsPerSample: 8,
                               samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                               colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    draw()
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

func centred(_ size: NSSize, in px: Int, fill: CGFloat) -> NSRect {
    let scale = min(CGFloat(px) * fill / size.width, CGFloat(px) * fill / size.height)
    let (w, h) = (size.width * scale, size.height * scale)
    return NSRect(x: (CGFloat(px) - w) / 2, y: (CGFloat(px) - h) / 2, width: w, height: h)
}

func sfSymbol(_ name: String, _ color: NSColor?, px: Int) -> Data {
    guard var img = NSImage(systemSymbolName: name, accessibilityDescription: nil) else { fatalError("no SF Symbol \(name)") }
    var cfg = NSImage.SymbolConfiguration(pointSize: CGFloat(px) * 0.62, weight: .regular)
    if let c = color {
        // Filled circle/octagon symbols: white glyph on the colour, like system alerts.
        let twoTone = name.hasSuffix("circle.fill") || name.hasSuffix("octagon.fill")
        cfg = cfg.applying(.init(paletteColors: twoTone ? [.white, c] : [c]))
    }
    img = img.withSymbolConfiguration(cfg)!
    return canvas(px) { img.draw(in: centred(img.size, in: px, fill: 0.86)) }
}

func material(_ stem: String) -> NSImage {
    let base = stem.replacingOccurrences(of: "_fill1", with: "")
    let url = URL(string: "https://raw.githubusercontent.com/google/material-design-icons/master/symbols/web/\(base)/materialsymbolsrounded/\(stem)_24px.svg")!
    guard let data = try? Data(contentsOf: url), let img = NSImage(data: data) else { fatalError("could not fetch \(stem)") }
    return img
}

func tinted(_ img: NSImage, _ color: NSColor, px: Int) -> Data {
    canvas(px) {
        let r = NSRect(x: 0, y: 0, width: px, height: px)
        img.draw(in: r)
        color.setFill()
        r.fill(using: .sourceAtop)
    }
}

for (id, sf, ms, color) in states {
    try! sfSymbol(sf, color, px: 44).write(to: root.appendingPathComponent("mac-\(id).png"))
    let icon = material(ms)
    if let c = color {
        try! tinted(icon, c, px: 32).write(to: root.appendingPathComponent("win-\(id).png"))
    } else {
        try! tinted(icon, .white, px: 32).write(to: root.appendingPathComponent("win-\(id)-dark.png"))
        try! tinted(icon, .black, px: 32).write(to: root.appendingPathComponent("win-\(id)-light.png"))
    }
    print("\(id): \(sf) / \(ms)")
}
