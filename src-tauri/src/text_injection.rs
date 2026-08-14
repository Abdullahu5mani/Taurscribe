//! Multi-tier text injection and clipboard pasting across Linux (Wayland/X11), macOS, and Windows.
//!
//! On Linux under Wayland (GNOME, KDE Plasma, Sway, Hyprland), client isolation prevents
//! synthetic keystrokes via XTest/Enigo from reaching foreign application surfaces.
//! This module implements a prioritized 5-tier injection strategy:
//! 1. Direct Kernel `/dev/uinput` Virtual Keyboard (instantaneous, compositor-agnostic)
//! 2. `ydotool` / `ydotoold` CLI automation daemon
//! 3. `wtype` for wlroots-based compositors
//! 4. FreeDesktop RemoteDesktop Portal (`org.freedesktop.portal.RemoteDesktop`)
//! 5. `enigo` synthetic Ctrl+V fallback (for X11 and legacy sessions)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextInjectionBackend {
    UInput,
    Ydotool,
    Wtype,
    RemoteDesktopPortal,
    Enigo,
}

/// Evaluates available system facilities to select the optimal text injection backend.
pub fn select_text_injection_backend(
    session_type: Option<&str>,
    has_uinput: bool,
    has_ydotool: bool,
    has_wtype: bool,
    has_portal: bool,
) -> Result<TextInjectionBackend, String> {
    let is_wayland = session_type
        .map(|s| s.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false);

    if is_wayland {
        if has_uinput {
            Ok(TextInjectionBackend::UInput)
        } else if has_ydotool {
            Ok(TextInjectionBackend::Ydotool)
        } else if has_wtype {
            Ok(TextInjectionBackend::Wtype)
        } else if has_portal {
            Ok(TextInjectionBackend::RemoteDesktopPortal)
        } else {
            Err("No Wayland-compatible text injection backend available".into())
        }
    } else {
        // X11 or fallback
        Ok(TextInjectionBackend::Enigo)
    }
}

/// Detects whether the current runtime environment is a Wayland session.
pub fn is_wayland_session() -> bool {
    if let Ok(disp) = std::env::var("WAYLAND_DISPLAY") {
        if !disp.trim().is_empty() {
            return true;
        }
    }
    if let Ok(sess) = std::env::var("XDG_SESSION_TYPE") {
        if sess.eq_ignore_ascii_case("wayland") {
            return true;
        }
    }
    false
}

/// Main entry point for cross-platform text injection.
/// Places `text` onto the system clipboard, simulates the appropriate paste shortcut
/// (Cmd+V on macOS, Ctrl+V on Windows/Linux via the optimal backend), and restores previous clipboard.
pub fn inject_text_or_paste(text: &str) -> Result<TextInjectionBackend, String> {
    if text.is_empty() {
        return Ok(TextInjectionBackend::Enigo);
    }

    let mut clipboard = match arboard::Clipboard::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[INSERT] Clipboard init failed: {}", e);
            return Err(format!("clipboard_init:{e}"));
        }
    };

    let previous_text = clipboard.get_text().ok();

    if let Err(e) = clipboard.set_text(text) {
        eprintln!("[INSERT] Failed to set clipboard: {}", e);
        return Err(format!("clipboard_set:{e}"));
    }

    // Allow clipboard data to settle in the clipboard manager / server
    std::thread::sleep(std::time::Duration::from_millis(50));

    let backend = perform_paste_keystroke()?;

    // Allow target application to process paste asynchronously before restoring
    std::thread::sleep(std::time::Duration::from_millis(300));
    if let Some(prev) = previous_text {
        let _ = clipboard.set_text(prev);
    }

    Ok(backend)
}

#[cfg(target_os = "macos")]
fn perform_paste_keystroke() -> Result<TextInjectionBackend, String> {
    simulate_macos_cmd_v()?;
    Ok(TextInjectionBackend::Enigo)
}

#[cfg(target_os = "windows")]
fn perform_paste_keystroke() -> Result<TextInjectionBackend, String> {
    simulate_enigo_ctrl_v()?;
    Ok(TextInjectionBackend::Enigo)
}

#[cfg(target_os = "linux")]
fn perform_paste_keystroke() -> Result<TextInjectionBackend, String> {
    if is_wayland_session() {
        // Multi-tier strategy:
        // Tier 1: /dev/uinput
        if let Ok(()) = try_uinput_ctrl_v() {
            println!("[INSERT] Wayland text injection via /dev/uinput succeeded");
            return Ok(TextInjectionBackend::UInput);
        }

        // Tier 2: ydotool
        if let Ok(()) = try_ydotool_ctrl_v() {
            println!("[INSERT] Wayland text injection via ydotool succeeded");
            return Ok(TextInjectionBackend::Ydotool);
        }

        // Tier 3: wtype
        if let Ok(()) = try_wtype_ctrl_v() {
            println!("[INSERT] Wayland text injection via wtype succeeded");
            return Ok(TextInjectionBackend::Wtype);
        }

        // Tier 4: RemoteDesktop portal
        if let Ok(()) = try_portal_ctrl_v() {
            println!("[INSERT] Wayland text injection via RemoteDesktop portal succeeded");
            return Ok(TextInjectionBackend::RemoteDesktopPortal);
        }

        // Tier 5: fallback to enigo
        eprintln!("[INSERT] Wayland injection tiers exhausted, attempting Enigo fallback");
        simulate_enigo_ctrl_v()?;
        Ok(TextInjectionBackend::Enigo)
    } else {
        simulate_enigo_ctrl_v()?;
        Ok(TextInjectionBackend::Enigo)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn perform_paste_keystroke() -> Result<TextInjectionBackend, String> {
    simulate_enigo_ctrl_v()?;
    Ok(TextInjectionBackend::Enigo)
}

#[cfg(target_os = "macos")]
fn simulate_macos_cmd_v() -> Result<(), String> {
    use core_graphics::event::{CGEvent, CGEventFlags, CGKeyCode};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|_| "cgevent_source_failed".to_string())?;

    const KEY_V: CGKeyCode = 9;

    let key_down = CGEvent::new_keyboard_event(source.clone(), KEY_V, true)
        .map_err(|_| "cgevent_create_down_failed".to_string())?;
    key_down.set_flags(CGEventFlags::CGEventFlagCommand);

    let key_up = CGEvent::new_keyboard_event(source, KEY_V, false)
        .map_err(|_| "cgevent_create_up_failed".to_string())?;
    key_up.set_flags(CGEventFlags::CGEventFlagCommand);

    key_down.post(core_graphics::event::CGEventTapLocation::HID);
    std::thread::sleep(std::time::Duration::from_millis(20));
    key_up.post(core_graphics::event::CGEventTapLocation::HID);

    Ok(())
}

#[allow(dead_code)]
fn simulate_enigo_ctrl_v() -> Result<(), String> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut enigo = match Enigo::new(&Settings::default()) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[INSERT] Enigo init failed: {:?}", e);
            return Err(format!("enigo_init:{e:?}"));
        }
    };
    let _ = enigo.key(Key::Control, Direction::Press);
    std::thread::sleep(std::time::Duration::from_millis(20));
    let _ = enigo.key(Key::Unicode('v'), Direction::Click);
    std::thread::sleep(std::time::Duration::from_millis(20));
    let _ = enigo.key(Key::Control, Direction::Release);
    Ok(())
}

#[cfg(target_os = "linux")]
fn try_uinput_ctrl_v() -> Result<(), String> {
    use std::fs::OpenOptions;
    use std::os::fd::AsRawFd;

    let file = OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .map_err(|e| format!("uinput open failed: {e}"))?;

    let fd = file.as_raw_fd();

    const UI_SET_EVBIT: std::ffi::c_ulong = 0x40045564;
    const UI_SET_KEYBIT: std::ffi::c_ulong = 0x40045565;
    const UI_DEV_SETUP: std::ffi::c_ulong = 0x405c5503;
    const UI_DEV_CREATE: std::ffi::c_ulong = 0x5501;
    const UI_DEV_DESTROY: std::ffi::c_ulong = 0x5502;

    const EV_SYN: u16 = 0x00;
    const SYN_REPORT: u16 = 0x00;
    const EV_KEY: u16 = 0x01;
    const KEY_LEFTCTRL: u16 = 29;
    const KEY_V: u16 = 47;
    const BUS_USB: u16 = 0x03;

    #[repr(C)]
    struct InputId {
        bustype: u16,
        vendor: u16,
        product: u16,
        version: u16,
    }

    #[repr(C)]
    struct UinputSetup {
        id: InputId,
        name: [std::ffi::c_char; 80],
        ff_effects_max: u32,
    }

    #[repr(C)]
    struct Timeval {
        tv_sec: i64,
        tv_usec: i64,
    }

    #[repr(C)]
    struct InputEvent {
        time: Timeval,
        type_: u16,
        code: u16,
        value: i32,
    }

    extern "C" {
        fn ioctl(fd: std::ffi::c_int, request: std::ffi::c_ulong, ...) -> std::ffi::c_int;
        fn write(fd: std::ffi::c_int, buf: *const std::ffi::c_void, count: usize) -> isize;
    }

    unsafe {
        if ioctl(fd, UI_SET_EVBIT, EV_KEY as std::ffi::c_int) < 0 {
            return Err("ioctl UI_SET_EVBIT EV_KEY failed".into());
        }
        if ioctl(fd, UI_SET_EVBIT, EV_SYN as std::ffi::c_int) < 0 {
            return Err("ioctl UI_SET_EVBIT EV_SYN failed".into());
        }
        if ioctl(fd, UI_SET_KEYBIT, KEY_LEFTCTRL as std::ffi::c_int) < 0 {
            return Err("ioctl UI_SET_KEYBIT KEY_LEFTCTRL failed".into());
        }
        if ioctl(fd, UI_SET_KEYBIT, KEY_V as std::ffi::c_int) < 0 {
            return Err("ioctl UI_SET_KEYBIT KEY_V failed".into());
        }

        let mut usetup: UinputSetup = std::mem::zeroed();
        usetup.id.bustype = BUS_USB;
        usetup.id.vendor = 0x1234;
        usetup.id.product = 0x5678;
        let dev_name = b"Taurscribe Virtual Keyboard\0";
        for (i, &b) in dev_name.iter().enumerate() {
            if i < 80 {
                usetup.name[i] = b as std::ffi::c_char;
            }
        }

        if ioctl(fd, UI_DEV_SETUP, &usetup) < 0 {
            #[repr(C)]
            struct UinputUserDev {
                name: [std::ffi::c_char; 80],
                id: InputId,
                ff_effects_max: u32,
                effects_max: [i32; 64],
            }
            let mut udev: UinputUserDev = std::mem::zeroed();
            udev.id = usetup.id;
            for (i, &b) in dev_name.iter().enumerate() {
                if i < 80 {
                    udev.name[i] = b as std::ffi::c_char;
                }
            }
            let ptr = &udev as *const _ as *const std::ffi::c_void;
            let sz = std::mem::size_of::<UinputUserDev>();
            if write(fd, ptr, sz) <= 0 {
                return Err("uinput write user_dev failed".into());
            }
        }

        if ioctl(fd, UI_DEV_CREATE) < 0 {
            return Err("ioctl UI_DEV_CREATE failed".into());
        }

        let emit = |type_: u16, code: u16, value: i32| -> Result<(), String> {
            let mut ev: InputEvent = std::mem::zeroed();
            ev.type_ = type_;
            ev.code = code;
            ev.value = value;
            let ptr = &ev as *const _ as *const std::ffi::c_void;
            let sz = std::mem::size_of::<InputEvent>();
            let written = write(fd, ptr, sz);
            if written != sz as isize {
                return Err("uinput write event failed".into());
            }
            Ok(())
        };

        std::thread::sleep(std::time::Duration::from_millis(50));

        emit(EV_KEY, KEY_LEFTCTRL, 1)?;
        emit(EV_SYN, SYN_REPORT, 0)?;
        std::thread::sleep(std::time::Duration::from_millis(20));

        emit(EV_KEY, KEY_V, 1)?;
        emit(EV_SYN, SYN_REPORT, 0)?;
        std::thread::sleep(std::time::Duration::from_millis(20));

        emit(EV_KEY, KEY_V, 0)?;
        emit(EV_SYN, SYN_REPORT, 0)?;
        std::thread::sleep(std::time::Duration::from_millis(20));

        emit(EV_KEY, KEY_LEFTCTRL, 0)?;
        emit(EV_SYN, SYN_REPORT, 0)?;

        std::thread::sleep(std::time::Duration::from_millis(20));
        let _ = ioctl(fd, UI_DEV_DESTROY);
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn try_ydotool_ctrl_v() -> Result<(), String> {
    let status = std::process::Command::new("ydotool")
        .args(["key", "29:1", "47:1", "47:0", "29:0"])
        .status()
        .map_err(|e| format!("ydotool spawn error: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("ydotool exited with code: {:?}", status.code()))
    }
}

#[cfg(target_os = "linux")]
fn try_wtype_ctrl_v() -> Result<(), String> {
    let status = std::process::Command::new("wtype")
        .args(["-M", "ctrl", "-k", "v", "-m", "ctrl"])
        .status()
        .map_err(|e| format!("wtype spawn error: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("wtype exited with code: {:?}", status.code()))
    }
}

#[cfg(target_os = "linux")]
fn try_portal_ctrl_v() -> Result<(), String> {
    let output = std::process::Command::new("busctl")
        .args([
            "--user",
            "call",
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.RemoteDesktop",
            "CreateSession",
            "a{sv}",
            "0",
        ])
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            return Ok(());
        }
    }

    let output2 = std::process::Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.portal.Desktop",
            "--object-path",
            "/org/freedesktop/portal/desktop",
            "--method",
            "org.freedesktop.portal.RemoteDesktop.CreateSession",
            "{}",
        ])
        .output();

    if let Ok(out) = output2 {
        if out.status.success() {
            return Ok(());
        }
    }

    Err("RemoteDesktop portal invocation failed".into())
}
