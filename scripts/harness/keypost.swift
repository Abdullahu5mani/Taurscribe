// Posts key presses to one process only (CGEventPostToPid), so a test can drive
// a native popup menu or a shortcut in Taurscribe without typing into whatever
// else is frontmost.
//
// Usage: keypost [--activate] <pid> <key> [<key> ...]
//   key: a single character, or return | escape | down | up | tab | space,
//   optionally prefixed with modifiers: cmd+, shift+, alt+, ctrl+ (e.g. cmd+shift+g)

import AppKit
import CoreGraphics
import Foundation

let named: [String: CGKeyCode] = [
    "return": 36, "escape": 53, "down": 125, "up": 126, "left": 123, "right": 124,
    "tab": 48, "space": 49, "delete": 51,
]
let letters: [Character: CGKeyCode] = [
    "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8, "v": 9, "b": 11, "q": 12,
    "w": 13, "e": 14, "r": 15, "y": 16, "t": 17, "1": 18, "2": 19, "3": 20, "4": 21, "6": 22, "5": 23,
    "9": 25, "7": 26, "8": 28, "0": 29, "o": 31, "u": 32, "i": 34, "p": 35, "l": 37, "j": 38, "k": 40,
    "n": 45, "m": 46, "/": 44, ".": 47, "\\": 42, ";": 41, "-": 27, "=": 24, ",": 43, "'": 39,
]

var args = Array(CommandLine.arguments.dropFirst())
// --activate: bring the target app to the front first, so a sheet or panel it
// shows is the key window and receives the keys.
let activate = args.contains("--activate")
args.removeAll { $0 == "--activate" }
guard args.count >= 2, let pid = pid_t(args[0]) else {
    FileHandle.standardError.write("usage: keypost <pid> <key> [<key> ...]\n".data(using: .utf8)!)
    exit(2)
}

if activate, let app = NSRunningApplication(processIdentifier: pid) {
    app.activate()
    usleep(500_000)
}

let source = CGEventSource(stateID: .hidSystemState)
for spec in args.dropFirst() {
    // "text:<anything>" types the string verbatim (any characters).
    if spec.hasPrefix("text:") {
        let text = Array(String(spec.dropFirst(5)).utf16)
        for chunk in stride(from: 0, to: text.count, by: 16) {
            let piece = Array(text[chunk..<min(chunk + 16, text.count)])
            for down in [true, false] {
                let ev = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: down)!
                ev.keyboardSetUnicodeString(stringLength: piece.count, unicodeString: piece)
                ev.postToPid(pid)
                usleep(20_000)
            }
        }
        usleep(120_000)
        continue
    }
    var parts = spec.lowercased().split(separator: "+").map(String.init)
    let keyName = parts.removeLast()
    var flags: CGEventFlags = []
    for m in parts {
        switch m {
        case "cmd": flags.insert(.maskCommand)
        case "shift": flags.insert(.maskShift)
        case "alt", "opt": flags.insert(.maskAlternate)
        case "ctrl": flags.insert(.maskControl)
        default: break
        }
    }
    guard let code = named[keyName] ?? (keyName.count == 1 ? letters[keyName.first!] : nil) else {
        FileHandle.standardError.write("unknown key \(spec)\n".data(using: .utf8)!)
        exit(3)
    }
    for down in [true, false] {
        let ev = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: down)!
        ev.flags = flags
        ev.postToPid(pid)
        usleep(40_000)
    }
    usleep(120_000)
}
print("{\"posted\": \(args.count - 1)}")
