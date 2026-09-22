// Accessibility observer/actuator for native macOS apps.
//
// Taurscribe's UI lives in a WKWebView and the host's Google Meet runs in the
// user's real Chrome; neither can be driven over CDP. Both publish their DOM
// through the macOS accessibility tree, so this walks that tree and prints the
// same indexed-element JSON shape snapshot.js produces for CDP pages. Decider can
// then read (and act on) a desktop window exactly like a web page.
//
// Usage:
//   ax_snapshot snapshot <pid> [--window <substring>]
//   ax_snapshot press    <pid> <index> [--window <substring>] [--expect <element text>]
//   ax_snapshot setvalue <pid> <index> <text> [--window <substring>] [--expect <element text>]
//   ax_snapshot windows  <pid>
//   ax_snapshot windowid <pid>
//   ax_snapshot frame    <pid> [--window <substring>]
//
// Indices are stable between calls as long as the UI has not changed, because
// every command performs the same deterministic walk.

import ApplicationServices
import Foundation

func fail(_ msg: String, _ code: Int32) -> Never {
    FileHandle.standardError.write((msg + "\n").data(using: .utf8)!)
    exit(code)
}

func attr(_ el: AXUIElement, _ name: String) -> AnyObject? {
    var value: AnyObject?
    guard AXUIElementCopyAttributeValue(el, name as CFString, &value) == .success else { return nil }
    return value
}

func str(_ el: AXUIElement, _ name: String) -> String {
    guard let v = attr(el, name) else { return "" }
    if let s = v as? String { return s }
    if let n = v as? NSNumber { return n.stringValue }
    return ""
}

struct Element: Codable {
    let index: Int
    let role: String
    let text: String
    let value: String
    let disabled: Bool
    let checked: Bool
}

// Roles worth reporting: interactive controls plus text, so Decider can read labels
// like "Active call detected: Google Meet" whether they sit on a button or a span.
let reportRoles: Set<String> = [
    "AXButton", "AXCheckBox", "AXRadioButton", "AXPopUpButton", "AXMenuButton",
    "AXTextField", "AXTextArea", "AXLink", "AXStaticText", "AXHeading", "AXTab",
    "AXSwitch", "AXMenuItem", "AXProgressIndicator",
]
let textRoles: Set<String> = ["AXStaticText", "AXHeading"]

var args = Array(CommandLine.arguments.dropFirst())
var windowFilter: String? = nil
if let i = args.firstIndex(of: "--window"), i + 1 < args.count {
    windowFilter = args[i + 1].lowercased()
    args.removeSubrange(i...(i + 1))
}
// press only if the element at the index still carries this text (the UI may
// have changed since the snapshot the index came from).
var expectText: String? = nil
if let i = args.firstIndex(of: "--expect"), i + 1 < args.count {
    expectText = args[i + 1]
    args.removeSubrange(i...(i + 1))
}
guard args.count >= 2, let pid = pid_t(args[1]) else {
    fail("usage: ax_snapshot snapshot|press|windows <pid> [index] [--window <substring>]", 2)
}
let command = args[0]

if !AXIsProcessTrusted() {
    fail("accessibility permission not granted to this process", 3)
}

let app = AXUIElementCreateApplication(pid)
AXUIElementSetMessagingTimeout(app, 4.0)
// Web views only publish their DOM subtree once an assistive client asks for it.
AXUIElementSetAttributeValue(app, "AXManualAccessibility" as CFString, kCFBooleanTrue)
AXUIElementSetAttributeValue(app, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue)

let allWindows = (attr(app, kAXWindowsAttribute) as? [AXUIElement]) ?? []

if command == "windowid" {
    // CGWindowID of the app's largest on-screen window, for `screencapture -l`
    // (captures the window itself even when other windows cover it).
    let info = (CGWindowListCopyWindowInfo([.optionAll, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]]) ?? []
    let mine = info.filter { ($0[kCGWindowOwnerPID as String] as? Int32) == pid && ($0[kCGWindowLayer as String] as? Int) == 0 }
    let best = mine.max { a, b in
        let ba = a[kCGWindowBounds as String] as? [String: CGFloat] ?? [:]
        let bb = b[kCGWindowBounds as String] as? [String: CGFloat] ?? [:]
        return (ba["Width"] ?? 0) * (ba["Height"] ?? 0) < (bb["Width"] ?? 0) * (bb["Height"] ?? 0)
    }
    guard let id = best?[kCGWindowNumber as String] as? Int else { fail("no window for pid", 4) }
    print(id)
    exit(0)
}

if command == "menupick" {
    // Picks an item from an open popup menu (a <select> in the web view opens a
    // native menu that is not part of any window).
    guard args.count >= 3 else { fail("usage: ax_snapshot menupick <pid> <item title>", 2) }
    let want = args[2]
    func findMenus(_ el: AXUIElement, depth: Int) -> [AXUIElement] {
        if depth > 6 { return [] }
        var out: [AXUIElement] = []
        if str(el, kAXRoleAttribute) == "AXMenu" { out.append(el) }
        for c in (attr(el, kAXChildrenAttribute) as? [AXUIElement]) ?? [] { out += findMenus(c, depth: depth + 1) }
        return out
    }
    var titles: [String] = []
    // An open <select> hangs its menu off the focused popup button; search there
    // before the app's menu bar (which also contains AXMenus).
    var roots: [AXUIElement] = []
    if let focused = attr(app, kAXFocusedUIElementAttribute) { roots.append(focused as! AXUIElement) }
    roots.append(app)
    var menus: [AXUIElement] = []
    for r in roots { menus += findMenus(r, depth: 0) }
    for menu in menus {
        for item in (attr(menu, kAXChildrenAttribute) as? [AXUIElement]) ?? [] {
            let t = str(item, kAXTitleAttribute)
            titles.append(t)
            if t == want {
                let err = AXUIElementPerformAction(item, kAXPressAction as CFString)
                if err != .success { fail("menu item press failed: \(err.rawValue)", 6) }
                print("{\"picked\": \"\(want)\"}")
                exit(0)
            }
        }
    }
    fail("no open menu item '\(want)' (saw: \(titles.joined(separator: ", ")))", 5)
}

if command == "tray" {
    // The app's menu-bar extra (tray icon): list its menu, or press an item.
    //   ax_snapshot tray <pid> [<item title>]
    guard let bar = attr(app, "AXExtrasMenuBar") else { fail("no tray icon (AXExtrasMenuBar)", 4) }
    let items = (attr(bar as! AXUIElement, kAXChildrenAttribute) as? [AXUIElement]) ?? []
    guard let icon = items.first else { fail("tray has no items", 4) }
    AXUIElementPerformAction(icon, kAXPressAction as CFString)
    usleep(500_000)
    var titles: [String] = []
    var target: AXUIElement? = nil
    for menu in (attr(icon, kAXChildrenAttribute) as? [AXUIElement]) ?? [] {
        for item in (attr(menu, kAXChildrenAttribute) as? [AXUIElement]) ?? [] {
            let t = str(item, kAXTitleAttribute)
            let enabled = (attr(item, kAXEnabledAttribute) as? Bool) ?? true
            titles.append(enabled ? t : "\(t) (disabled)")
            if args.count >= 3 && t == args[2] { target = item }
        }
    }
    if args.count >= 3 {
        guard let item = target else {
            // close the menu again before failing
            for menu in (attr(icon, kAXChildrenAttribute) as? [AXUIElement]) ?? [] { AXUIElementPerformAction(menu, kAXCancelAction as CFString) }
            fail("no tray item '\(args[2])' (saw: \(titles.joined(separator: ", ")))", 5)
        }
        AXUIElementPerformAction(item, kAXPressAction as CFString)
        print("{\"pressed\": \"\(args[2])\"}")
    } else {
        for menu in (attr(icon, kAXChildrenAttribute) as? [AXUIElement]) ?? [] { AXUIElementPerformAction(menu, kAXCancelAction as CFString) }
        FileHandle.standardOutput.write(try! JSONEncoder().encode(titles))
    }
    exit(0)
}

if command == "trayinfo" {
    // The tray icon's menu-bar title and tooltip, without opening its menu:
    //   ax_snapshot trayinfo <pid>  ->  {"title", "help", "description", "frame": [x, y, w, h]}
    guard let bar = attr(app, "AXExtrasMenuBar") else { fail("no tray icon (AXExtrasMenuBar)", 4) }
    guard let icon = ((attr(bar as! AXUIElement, kAXChildrenAttribute) as? [AXUIElement]) ?? []).first else { fail("tray has no items", 4) }
    var point = CGPoint.zero, size = CGSize.zero
    if let p = attr(icon, kAXPositionAttribute) { AXValueGetValue(p as! AXValue, .cgPoint, &point) }
    if let s = attr(icon, kAXSizeAttribute) { AXValueGetValue(s as! AXValue, .cgSize, &size) }
    let info: [String: Any] = ["title": str(icon, kAXTitleAttribute), "help": str(icon, kAXHelpAttribute),
                "description": str(icon, kAXDescriptionAttribute),
                "frame": [Int(point.x), Int(point.y), Int(size.width), Int(size.height)]]
    FileHandle.standardOutput.write(try JSONSerialization.data(withJSONObject: info))
    exit(0)
}

if command == "menucancel" {
    // Closes an open popup menu without choosing anything.
    var closed = 0
    func cancelMenus(_ el: AXUIElement, depth: Int) {
        if depth > 8 { return }
        if str(el, kAXRoleAttribute) == "AXMenu" {
            if AXUIElementPerformAction(el, kAXCancelAction as CFString) == .success { closed += 1 }
            return
        }
        for c in (attr(el, kAXChildrenAttribute) as? [AXUIElement]) ?? [] { cancelMenus(c, depth: depth + 1) }
    }
    if let focused = attr(app, kAXFocusedUIElementAttribute) { cancelMenus(focused as! AXUIElement, depth: 0) }
    print("{\"closed\": \(closed)}")
    exit(0)
}

if command == "cgwindows" {
    // On-screen windows of the app (includes panels the AX tree omits, e.g. the
    // recording overlay): [{layer, w, h, name}].
    let info = (CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]]) ?? []
    let mine = info.filter { ($0[kCGWindowOwnerPID as String] as? Int32) == pid }.map { w -> [String: Any] in
        let b = w[kCGWindowBounds as String] as? [String: CGFloat] ?? [:]
        return ["layer": w[kCGWindowLayer as String] as? Int ?? 0, "w": Int(b["Width"] ?? 0), "h": Int(b["Height"] ?? 0),
                "name": w[kCGWindowName as String] as? String ?? ""]
    }
    FileHandle.standardOutput.write(try JSONSerialization.data(withJSONObject: mine))
    exit(0)
}

if command == "windows" {
    let titles = allWindows.map { str($0, kAXTitleAttribute) }
    FileHandle.standardOutput.write(try JSONEncoder().encode(titles))
    exit(0)
}

// With a filter, only the FIRST (frontmost) matching window: two windows of one
// app can share a title fragment, and mixing their controls lets a click land in
// the wrong one.
let windows: [AXUIElement] = {
    guard let f = windowFilter else { return allWindows }
    return allWindows.first { str($0, kAXTitleAttribute).lowercased().contains(f) }.map { [$0] } ?? []
}()
if windows.isEmpty {
    fail("no window matched" + (windowFilter.map { " '\($0)'" } ?? ""), 4)
}

if command == "frame" {
    var point = CGPoint.zero
    var size = CGSize.zero
    if let p = attr(windows[0], kAXPositionAttribute) { AXValueGetValue(p as! AXValue, .cgPoint, &point) }
    if let s = attr(windows[0], kAXSizeAttribute) { AXValueGetValue(s as! AXValue, .cgSize, &size) }
    print("{\"x\": \(Int(point.x)), \"y\": \(Int(point.y)), \"w\": \(Int(size.width)), \"h\": \(Int(size.height))}")
    exit(0)
}

/// Concatenated static text beneath an element (for headings without a title).
func childText(_ el: AXUIElement, depth: Int = 0) -> String {
    if depth > 4 { return "" }
    var parts: [String] = []
    for c in (attr(el, kAXChildrenAttribute) as? [AXUIElement]) ?? [] {
        if str(c, kAXRoleAttribute) == "AXStaticText" {
            parts.append(str(c, kAXValueAttribute))
        } else {
            parts.append(childText(c, depth: depth + 1))
        }
    }
    return parts.filter { !$0.isEmpty }.joined(separator: " ")
}

var elements: [Element] = []
var nodes: [AXUIElement] = []

func walk(_ el: AXUIElement, depth: Int) {
    if depth > 60 { return }
    let role = str(el, kAXRoleAttribute)
    // Labelled groups (aria-label on a container, e.g. a model card) are worth
    // reporting; unlabelled layout groups are not.
    let labelledGroup = role == "AXGroup" && !str(el, kAXDescriptionAttribute).isEmpty
    if reportRoles.contains(role) || labelledGroup {
        var text: String
        var value: String
        if role == "AXHeading" {
            // A heading's AXValue is its level ("3"); its words are in the title,
            // the description, or its text children.
            text = [str(el, kAXTitleAttribute), str(el, kAXDescriptionAttribute)].first { !$0.isEmpty } ?? childText(el)
            value = ""
        } else if textRoles.contains(role) {
            text = str(el, kAXValueAttribute)
            if text.isEmpty { text = str(el, kAXDescriptionAttribute) }
            if text.isEmpty { text = str(el, kAXTitleAttribute) }
            value = ""
        } else {
            text = [str(el, kAXDescriptionAttribute), str(el, kAXTitleAttribute), str(el, kAXHelpAttribute),
                    str(el, "AXPlaceholderValue")]
                .first { !$0.isEmpty } ?? ""
            value = str(el, kAXValueAttribute)
        }
        text = text.replacingOccurrences(of: "\\s+", with: " ", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        if text.count > 160 { text = String(text.prefix(160)) }
        // WebKit often echoes a label as a static-text child right after the
        // control; drop only that immediate repeat. A global dedup would also
        // merge distinct readings that happen to match (two meters both at "0%").
        let isDupText = textRoles.contains(role) && elements.last?.text == text
        if (!text.isEmpty || !value.isEmpty) && !isDupText {
            let enabled = (attr(el, kAXEnabledAttribute) as? Bool) ?? true
            let checked = ["AXCheckBox", "AXSwitch", "AXRadioButton"].contains(role) && value == "1"
            elements.append(Element(
                index: elements.count + 1, role: role, text: text, value: value,
                disabled: !enabled, checked: checked))
            nodes.append(el)
        }
    }
    guard let children = attr(el, kAXChildrenAttribute) as? [AXUIElement] else { return }
    for child in children { walk(child, depth: depth + 1) }
}

for w in windows { walk(w, depth: 0) }

switch command {
case "snapshot":
    let table = elements.map { e -> String in
        var line = "[\(e.index)] \(e.role) '\(e.text)'"
        if !e.value.isEmpty { line += " · value: '\(e.value)'" }
        if e.disabled { line += " · disabled" }
        if e.checked { line += " · checked/active" }
        return line
    }.joined(separator: "\n")
    let out: [String: Any] = [
        "title": str(windows[0], kAXTitleAttribute),
        "pid": Int(pid),
        "elements": elements.map { e -> [String: Any] in
            ["index": e.index, "role": e.role, "tag": e.role, "type": "", "text": e.text,
             "value": e.value, "disabled": e.disabled, "checked": e.checked]
        },
        "table": table,
    ]
    FileHandle.standardOutput.write(try JSONSerialization.data(withJSONObject: out))
case "setvalue":
    // Types into a text field by setting its accessibility value (Chrome turns
    // this into a real input event for the page).
    guard args.count >= 4, let idx = Int(args[2]), idx >= 1, idx <= nodes.count else {
        fail("setvalue: index out of range (have \(nodes.count) elements)", 5)
    }
    if let want = expectText, !elements[idx - 1].text.hasPrefix(want) {
        fail("setvalue: element \(idx) is now '\(elements[idx - 1].text)', expected '\(want)'", 7)
    }
    let node = nodes[idx - 1]
    AXUIElementSetAttributeValue(node, kAXFocusedAttribute as CFString, kCFBooleanTrue)
    // Replace like a user would: select all existing text, then replace the
    // selection (setting AXValue on some web inputs appended to the old text).
    let current = str(node, kAXValueAttribute)
    var range = CFRange(location: 0, length: (current as NSString).length)
    var replaced = false
    if let rangeValue = AXValueCreate(.cfRange, &range),
       AXUIElementSetAttributeValue(node, kAXSelectedTextRangeAttribute as CFString, rangeValue) == .success,
       AXUIElementSetAttributeValue(node, kAXSelectedTextAttribute as CFString, args[3] as CFString) == .success {
        replaced = str(node, kAXValueAttribute) == args[3]
    }
    if !replaced {
        let err = AXUIElementSetAttributeValue(node, kAXValueAttribute as CFString, args[3] as CFString)
        if err != .success { fail("AXValue set failed: \(err.rawValue)", 6) }
    }
    print("{\"set\": \(idx)}")
case "select":
    // Selects the list/browser row holding element <index> (e.g. a file name in
    // an open panel): walks up to the AXRow and marks it selected.
    guard args.count >= 3, let idx = Int(args[2]), idx >= 1, idx <= nodes.count else {
        fail("select: index out of range (have \(nodes.count) elements)", 5)
    }
    // Two shapes: a table row (AXRow with settable AXSelected), or an item in a
    // list whose AXSelectedChildren is settable (the open panel's column browser).
    var child: AXUIElement = nodes[idx - 1]
    var hops = 0
    while hops < 8 {
        if str(child, kAXRoleAttribute) == "AXRow" {
            let err = AXUIElementSetAttributeValue(child, kAXSelectedAttribute as CFString, kCFBooleanTrue)
            if err != .success { fail("select failed: \(err.rawValue)", 6) }
            print("{\"selected\": \(idx)}"); exit(0)
        }
        guard let parentObj = attr(child, kAXParentAttribute) else { break }
        let parent = parentObj as! AXUIElement
        var settable = DarwinBoolean(false)
        AXUIElementIsAttributeSettable(parent, "AXSelectedChildren" as CFString, &settable)
        if settable.boolValue {
            let err = AXUIElementSetAttributeValue(parent, "AXSelectedChildren" as CFString, [child] as CFArray)
            if err != .success { fail("select failed: \(err.rawValue)", 6) }
            print("{\"selected\": \(idx)}"); exit(0)
        }
        child = parent
        hops += 1
    }
    fail("select: nothing selectable above element \(idx)", 6)
case "ancestry":
    guard args.count >= 3, let idx = Int(args[2]), idx >= 1, idx <= nodes.count else { fail("bad index", 5) }
    var cur: AXUIElement? = nodes[idx - 1]
    var chain: [String] = []
    var n = 0
    while let c = cur, n < 12 {
        var names: CFArray?
        AXUIElementCopyAttributeNames(c, &names)
        let settable = ["AXSelected", "AXSelectedRows", "AXSelectedChildren"].filter { name in
            var ok = DarwinBoolean(false); AXUIElementIsAttributeSettable(c, name as CFString, &ok); return ok.boolValue }
        chain.append("\(str(c, kAXRoleAttribute))[\(str(c, kAXSubroleAttribute))] settable=\(settable)")
        cur = attr(c, kAXParentAttribute).map { $0 as! AXUIElement }
        n += 1
    }
    print(chain.joined(separator: "\n"))
case "scrollto":
    guard args.count >= 3, let idx = Int(args[2]), idx >= 1, idx <= nodes.count else {
        fail("scrollto: index out of range (have \(nodes.count) elements)", 5)
    }
    let err = AXUIElementPerformAction(nodes[idx - 1], "AXScrollToVisible" as CFString)
    if err != .success { fail("AXScrollToVisible failed: \(err.rawValue)", 6) }
    print("{\"scrolled\": \(idx)}")
case "press":
    guard args.count >= 3, let idx = Int(args[2]), idx >= 1, idx <= nodes.count else {
        fail("press: index out of range (have \(nodes.count) elements)", 5)
    }
    if let want = expectText, !elements[idx - 1].text.hasPrefix(want) {
        fail("press: element \(idx) is now '\(elements[idx - 1].text)', expected '\(want)'", 7)
    }
    let err = AXUIElementPerformAction(nodes[idx - 1], kAXPressAction as CFString)
    if err != .success { fail("AXPress failed: \(err.rawValue)", 6) }
    print("{\"pressed\": \(idx)}")
default:
    fail("unknown command \(command)", 2)
}
