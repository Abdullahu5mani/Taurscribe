//! Read Google Meet's visible call controls without enabling browser scripting.
//! An unreadable or localized page remains unknown; only explicit preview/end
//! controls suppress the audio detector's result.

use accessibility_sys::{
    kAXErrorSuccess, AXUIElementCopyAttributeValue, AXUIElementCreateApplication,
    AXUIElementRef, AXUIElementSetAttributeValue, AXUIElementSetMessagingTimeout,
};
use core_foundation::{
    array::CFArray,
    base::{CFType, TCFType},
    boolean::CFBoolean,
    string::CFString,
    url::CFURL,
};
use std::{collections::HashMap, sync::{LazyLock, Mutex}, time::{Duration, Instant}};

type Rooms = HashMap<String, Option<bool>>;
static CACHE: LazyLock<Mutex<HashMap<u32, (Instant, Rooms)>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn state(pid: u32, url: &str) -> Option<bool> {
    if let Some((at, rooms)) = CACHE.lock().ok()?.get(&pid) {
        if at.elapsed() < Duration::from_secs(1) {
            return rooms.get(url).copied().flatten();
        }
    }
    let rooms = observe(pid);
    let result = rooms.get(url).copied().flatten();
    if let Ok(mut cache) = CACHE.lock() {
        cache.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(30));
        cache.insert(pid, (Instant::now(), rooms));
    }
    result
}

fn controls_state(labels: &[String]) -> Option<bool> {
    if labels.iter().any(|s| s.to_lowercase().starts_with("leave call")) {
        return Some(true);
    }
    labels.iter().any(|s| matches!(s.trim().to_lowercase().as_str(),
        "join now" | "join now without camera" | "ask to join" | "rejoin" | "return to home screen"
    )).then_some(false)
}

fn attribute(element: &CFType, name: &str) -> Option<CFType> {
    let key = CFString::new(name);
    let mut value = std::ptr::null();
    // CopyAttributeValue returns an owned CF object. CFType releases it on drop;
    // downcasts below check its runtime type before reading strings/arrays/URLs.
    unsafe {
        let err = AXUIElementCopyAttributeValue(element.as_CFTypeRef() as AXUIElementRef, key.as_concrete_TypeRef(), &mut value);
        if err != kAXErrorSuccess || value.is_null() { return None; }
        Some(CFType::wrap_under_create_rule(value))
    }
}

fn string_attribute(element: &CFType, name: &str) -> Option<String> {
    attribute(element, name)?.downcast::<CFString>().map(|s| s.to_string())
}

fn observe(pid: u32) -> Rooms {
    let mut rooms = HashMap::new();
    let app = unsafe {
        let raw = AXUIElementCreateApplication(pid as i32);
        if raw.is_null() { return rooms; }
        AXUIElementSetMessagingTimeout(raw, 0.1);
        for key in ["AXManualAccessibility", "AXEnhancedUserInterface"] {
            AXUIElementSetAttributeValue(raw, CFString::new(key).as_concrete_TypeRef(), CFBoolean::true_value().as_CFTypeRef());
        }
        CFType::wrap_under_create_rule(raw as _)
    };
    let deadline = Instant::now() + Duration::from_millis(800);
    let mut queue = vec![(app, None::<String>, 0)];
    let mut buttons: HashMap<String, Vec<String>> = HashMap::new();
    let mut visited = 0;
    while let Some((element, mut room, depth)) = queue.pop() {
        visited += 1;
        if visited > 2000 || Instant::now() >= deadline { break; }
        if depth > 60 { continue; }
        let role = string_attribute(&element, "AXRole").unwrap_or_default();
        if role == "AXWebArea" {
            let url = attribute(&element, "AXURL").and_then(|v| {
                v.downcast::<CFURL>().map(|u| u.get_string().to_string())
                    .or_else(|| v.downcast::<CFString>().map(|s| s.to_string()))
            });
            room = url.filter(|u| super::classify_meeting_url(u) == Some("meet"));
            // Other pages and their potentially similar buttons are irrelevant.
            if room.is_none() { continue; }
        }
        if role == "AXButton" {
            if let Some(url) = &room {
                for name in ["AXTitle", "AXDescription"] {
                    if let Some(label) = string_attribute(&element, name).filter(|s| !s.is_empty()) {
                        buttons.entry(url.clone()).or_default().push(label);
                    }
                }
            }
        }
        for name in [if depth == 0 { "AXWindows" } else { "AXChildren" }] {
            if let Some(array) = attribute(&element, name).and_then(|v| v.downcast::<CFArray>()) {
                for index in (0..array.len()).rev() {
                    let Some(child) = array.get(index) else { continue; };
                    // The array owns its children; retain each until traversal
                    // finishes, even after the array itself is dropped.
                    let child = unsafe { CFType::wrap_under_get_rule(*child as _) };
                    queue.push((child, room.clone(), depth + 1));
                }
            }
        }
    }
    for (url, labels) in buttons {
        rooms.insert(url, controls_state(&labels));
    }
    rooms
}

#[cfg(test)]
mod tests {
    use super::*;
    fn labels(items: &[&str]) -> Vec<String> { items.iter().map(|s| s.to_string()).collect() }
    #[test]
    fn preview_and_left_pages_are_not_calls() {
        for button in ["Join now", "Join now without camera", "Ask to join", "Rejoin", "Return to home screen"] {
            assert_eq!(controls_state(&labels(&[button, "Turn off microphone"])), Some(false));
        }
    }
    #[test]
    fn a_live_call_with_an_admission_dialog_stays_live() {
        assert_eq!(controls_state(&labels(&["Leave call", "Ask to join"])), Some(true));
    }
    #[test]
    fn missing_or_localized_controls_remain_unknown() {
        assert_eq!(controls_state(&[]), None);
        assert_eq!(controls_state(&labels(&["Quitter l’appel", "Microphone"])), None);
    }
}
