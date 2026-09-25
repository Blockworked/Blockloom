//! Qt's native scan codes as the `KeyCode` names `PreviewInput::Key` carries,
//! so the game hears where a key sits rather than what the layout prints on
//! it: WASD stays WASD on AZERTY or Dvorak, and a right Shift is a right Shift.

/// The physical key behind a `nativeScanCode`, or `None` where the platform
/// gives no usable one (macOS, or a synthetic event) and the caller should
/// fall back to Qt's key name.
pub fn physical_key(scan_code: u32) -> Option<&'static str> {
    if scan_code == 0 {
        return None;
    }
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        // xcb and Wayland both hand over the XKB keycode: evdev's plus 8.
        scan_code.checked_sub(8).and_then(evdev)
    }
    #[cfg(target_os = "windows")]
    {
        // Set 1 scan code, with 0x100 for an E0-prefixed (extended) key.
        set1(scan_code)
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "windows")))]
    {
        None
    }
}

/// Linux input event codes (`linux/input-event-codes.h`).
#[cfg_attr(not(any(target_os = "linux", target_os = "freebsd")), allow(dead_code))]
fn evdev(code: u32) -> Option<&'static str> {
    Some(match code {
        1 => "Escape",
        2 => "Digit1",
        3 => "Digit2",
        4 => "Digit3",
        5 => "Digit4",
        6 => "Digit5",
        7 => "Digit6",
        8 => "Digit7",
        9 => "Digit8",
        10 => "Digit9",
        11 => "Digit0",
        12 => "Minus",
        13 => "Equal",
        14 => "Backspace",
        15 => "Tab",
        16 => "KeyQ",
        17 => "KeyW",
        18 => "KeyE",
        19 => "KeyR",
        20 => "KeyT",
        21 => "KeyY",
        22 => "KeyU",
        23 => "KeyI",
        24 => "KeyO",
        25 => "KeyP",
        26 => "BracketLeft",
        27 => "BracketRight",
        28 => "Enter",
        29 => "ControlLeft",
        30 => "KeyA",
        31 => "KeyS",
        32 => "KeyD",
        33 => "KeyF",
        34 => "KeyG",
        35 => "KeyH",
        36 => "KeyJ",
        37 => "KeyK",
        38 => "KeyL",
        39 => "Semicolon",
        40 => "Quote",
        41 => "Backquote",
        42 => "ShiftLeft",
        43 => "Backslash",
        44 => "KeyZ",
        45 => "KeyX",
        46 => "KeyC",
        47 => "KeyV",
        48 => "KeyB",
        49 => "KeyN",
        50 => "KeyM",
        51 => "Comma",
        52 => "Period",
        53 => "Slash",
        54 => "ShiftRight",
        55 => "NumpadMultiply",
        56 => "AltLeft",
        57 => "Space",
        58 => "CapsLock",
        59 => "F1",
        60 => "F2",
        61 => "F3",
        62 => "F4",
        63 => "F5",
        64 => "F6",
        65 => "F7",
        66 => "F8",
        67 => "F9",
        68 => "F10",
        69 => "NumLock",
        70 => "ScrollLock",
        71 => "Numpad7",
        72 => "Numpad8",
        73 => "Numpad9",
        74 => "NumpadSubtract",
        75 => "Numpad4",
        76 => "Numpad5",
        77 => "Numpad6",
        78 => "NumpadAdd",
        79 => "Numpad1",
        80 => "Numpad2",
        81 => "Numpad3",
        82 => "Numpad0",
        83 => "NumpadDecimal",
        86 => "IntlBackslash",
        87 => "F11",
        88 => "F12",
        89 => "IntlRo",
        96 => "NumpadEnter",
        97 => "ControlRight",
        98 => "NumpadDivide",
        99 => "PrintScreen",
        100 => "AltRight",
        102 => "Home",
        103 => "ArrowUp",
        104 => "PageUp",
        105 => "ArrowLeft",
        106 => "ArrowRight",
        107 => "End",
        108 => "ArrowDown",
        109 => "PageDown",
        110 => "Insert",
        111 => "Delete",
        117 => "NumpadEqual",
        119 => "Pause",
        121 => "NumpadComma",
        124 => "IntlYen",
        125 => "SuperLeft",
        126 => "SuperRight",
        127 => "ContextMenu",
        183 => "F13",
        184 => "F14",
        185 => "F15",
        186 => "F16",
        187 => "F17",
        188 => "F18",
        189 => "F19",
        190 => "F20",
        191 => "F21",
        192 => "F22",
        193 => "F23",
        194 => "F24",
        _ => return None,
    })
}

/// PC/AT set 1 scan codes as Windows reports them.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn set1(code: u32) -> Option<&'static str> {
    Some(match code {
        0x01 => "Escape",
        0x02 => "Digit1",
        0x03 => "Digit2",
        0x04 => "Digit3",
        0x05 => "Digit4",
        0x06 => "Digit5",
        0x07 => "Digit6",
        0x08 => "Digit7",
        0x09 => "Digit8",
        0x0A => "Digit9",
        0x0B => "Digit0",
        0x0C => "Minus",
        0x0D => "Equal",
        0x0E => "Backspace",
        0x0F => "Tab",
        0x10 => "KeyQ",
        0x11 => "KeyW",
        0x12 => "KeyE",
        0x13 => "KeyR",
        0x14 => "KeyT",
        0x15 => "KeyY",
        0x16 => "KeyU",
        0x17 => "KeyI",
        0x18 => "KeyO",
        0x19 => "KeyP",
        0x1A => "BracketLeft",
        0x1B => "BracketRight",
        0x1C => "Enter",
        0x1D => "ControlLeft",
        0x1E => "KeyA",
        0x1F => "KeyS",
        0x20 => "KeyD",
        0x21 => "KeyF",
        0x22 => "KeyG",
        0x23 => "KeyH",
        0x24 => "KeyJ",
        0x25 => "KeyK",
        0x26 => "KeyL",
        0x27 => "Semicolon",
        0x28 => "Quote",
        0x29 => "Backquote",
        0x2A => "ShiftLeft",
        0x2B => "Backslash",
        0x2C => "KeyZ",
        0x2D => "KeyX",
        0x2E => "KeyC",
        0x2F => "KeyV",
        0x30 => "KeyB",
        0x31 => "KeyN",
        0x32 => "KeyM",
        0x33 => "Comma",
        0x34 => "Period",
        0x35 => "Slash",
        0x36 => "ShiftRight",
        0x37 => "NumpadMultiply",
        0x38 => "AltLeft",
        0x39 => "Space",
        0x3A => "CapsLock",
        0x3B => "F1",
        0x3C => "F2",
        0x3D => "F3",
        0x3E => "F4",
        0x3F => "F5",
        0x40 => "F6",
        0x41 => "F7",
        0x42 => "F8",
        0x43 => "F9",
        0x44 => "F10",
        // Pause and NumLock share 0x45; only NumLock is extended.
        0x45 => "Pause",
        0x46 => "ScrollLock",
        0x47 => "Numpad7",
        0x48 => "Numpad8",
        0x49 => "Numpad9",
        0x4A => "NumpadSubtract",
        0x4B => "Numpad4",
        0x4C => "Numpad5",
        0x4D => "Numpad6",
        0x4E => "NumpadAdd",
        0x4F => "Numpad1",
        0x50 => "Numpad2",
        0x51 => "Numpad3",
        0x52 => "Numpad0",
        0x53 => "NumpadDecimal",
        0x56 => "IntlBackslash",
        0x57 => "F11",
        0x58 => "F12",
        0x59 => "NumpadEqual",
        0x64 => "F13",
        0x65 => "F14",
        0x66 => "F15",
        0x67 => "F16",
        0x68 => "F17",
        0x69 => "F18",
        0x6A => "F19",
        0x6B => "F20",
        0x6C => "F21",
        0x6D => "F22",
        0x6E => "F23",
        0x73 => "IntlRo",
        0x76 => "F24",
        0x7D => "IntlYen",
        0x7E => "NumpadComma",
        0x11C => "NumpadEnter",
        0x11D => "ControlRight",
        0x135 => "NumpadDivide",
        0x137 => "PrintScreen",
        0x138 => "AltRight",
        0x145 => "NumLock",
        0x147 => "Home",
        0x148 => "ArrowUp",
        0x149 => "PageUp",
        0x14B => "ArrowLeft",
        0x14D => "ArrowRight",
        0x14F => "End",
        0x150 => "ArrowDown",
        0x151 => "PageDown",
        0x152 => "Insert",
        0x153 => "Delete",
        0x15B => "SuperLeft",
        0x15C => "SuperRight",
        0x15D => "ContextMenu",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_named_by_where_they_sit() {
        assert_eq!(evdev(17), Some("KeyW"));
        assert_eq!(evdev(54), Some("ShiftRight"));
        assert_eq!(evdev(26), Some("BracketLeft"));
        assert_eq!(evdev(79), Some("Numpad1"));
        assert_eq!(set1(0x11), Some("KeyW"));
        assert_eq!(set1(0x1D), Some("ControlLeft"));
        assert_eq!(set1(0x11D), Some("ControlRight"));
        assert_eq!(set1(0x1C), Some("Enter"));
        assert_eq!(set1(0x11C), Some("NumpadEnter"));
        assert_eq!(physical_key(0), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_scan_codes_are_xkb_keycodes() {
        assert_eq!(physical_key(17 + 8), Some("KeyW"));
        assert_eq!(physical_key(3), None);
    }
}
