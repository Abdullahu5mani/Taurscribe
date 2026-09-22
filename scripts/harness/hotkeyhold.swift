// Holds a modifier chord system-wide (HID-level events, as a real keyboard would),
// for testing Taurscribe's global hold-to-record hotkey.
//
// Usage: hotkeyhold <seconds> [ctrl] [alt] [shift] [cmd]   (left-hand modifiers)
// Prints "down" once the keys are held, then "up" after releasing them.

import CoreGraphics
import Foundation

let args = Array(CommandLine.arguments.dropFirst())
guard let seconds = args.first.flatMap(Double.init) else {
    FileHandle.standardError.write("usage: hotkeyhold <seconds> ctrl alt ...\n".data(using: .utf8)!)
    exit(2)
}
let table: [String: (CGKeyCode, CGEventFlags)] = [
    "ctrl": (59, .maskControl), "alt": (58, .maskAlternate), "shift": (56, .maskShift), "cmd": (55, .maskCommand),
]
let keys = args.dropFirst().compactMap { table[$0.lowercased()] }
let source = CGEventSource(stateID: .hidSystemState)

var flags: CGEventFlags = []
for (code, flag) in keys {
    flags.insert(flag)
    let ev = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: true)!
    ev.type = .flagsChanged
    ev.flags = flags
    ev.post(tap: .cghidEventTap)
    usleep(60_000)
}
print("down"); fflush(stdout)
usleep(useconds_t(seconds * 1_000_000))
for (code, flag) in keys.reversed() {
    flags.remove(flag)
    let ev = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: false)!
    ev.type = .flagsChanged
    ev.flags = flags
    ev.post(tap: .cghidEventTap)
    usleep(60_000)
}
print("up")
