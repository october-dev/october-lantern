import Carbon.HIToolbox

/// The shortcuts offered in Settings for opening the composer.
enum HotKeyPreset: String, CaseIterable, Identifiable {
    case controlOptionSpace, optionSpace, commandShiftSpace, controlOptionL, none

    var id: String { rawValue }

    var label: String {
        switch self {
        case .controlOptionSpace: "⌃⌥Space"
        case .optionSpace: "⌥Space"
        case .commandShiftSpace: "⇧⌘Space"
        case .controlOptionL: "⌃⌥L"
        case .none: "None"
        }
    }

    var keyCode: UInt32? {
        switch self {
        case .controlOptionL: UInt32(kVK_ANSI_L)
        case .none: nil
        default: UInt32(kVK_Space)
        }
    }

    var modifiers: UInt32 {
        switch self {
        case .controlOptionSpace, .controlOptionL: UInt32(controlKey | optionKey)
        case .optionSpace: UInt32(optionKey)
        case .commandShiftSpace: UInt32(cmdKey | shiftKey)
        case .none: 0
        }
    }
}

/// The shortcuts offered in Settings for Point & Ask.
enum PointAskPreset: String, CaseIterable, Identifiable {
    case controlOptionP, controlOptionK, none

    var id: String { rawValue }

    var label: String {
        switch self {
        case .controlOptionP: "⌃⌥P"
        case .controlOptionK: "⌃⌥K"
        case .none: "None"
        }
    }

    var keyCode: UInt32? {
        switch self {
        case .controlOptionP: UInt32(kVK_ANSI_P)
        case .controlOptionK: UInt32(kVK_ANSI_K)
        case .none: nil
        }
    }

    var modifiers: UInt32 { UInt32(controlKey | optionKey) }
}

/// A global hotkey through Carbon's RegisterEventHotKey, which needs no Accessibility permission.
/// Each hotkey has its own id, so several can be registered; `onRelease` runs when the key is let
/// go (for press-and-hold shortcuts).
final class HotKey {
    private var ref: EventHotKeyRef?
    private var handler: EventHandlerRef?
    private let number: UInt32
    private let action: () -> Void
    private let onRelease: (() -> Void)?

    /// Defaults to ⌃⌥Space.
    init(
        keyCode: UInt32 = UInt32(kVK_Space), modifiers: UInt32 = UInt32(controlKey | optionKey), id number: UInt32 = 1,
        onRelease: (() -> Void)? = nil, action: @escaping () -> Void
    ) {
        self.number = number
        self.action = action
        self.onRelease = onRelease
        var specs = [
            EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed)),
            EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyReleased)),
        ]
        let me = Unmanaged.passUnretained(self).toOpaque()
        InstallEventHandler(GetApplicationEventTarget(), { _, event, userData in
            guard let userData, let event else { return OSStatus(eventNotHandledErr) }
            let key = Unmanaged<HotKey>.fromOpaque(userData).takeUnretainedValue()
            var hit = EventHotKeyID()
            GetEventParameter(
                event, EventParamName(kEventParamDirectObject), EventParamType(typeEventHotKeyID), nil,
                MemoryLayout<EventHotKeyID>.size, nil, &hit
            )
            // Every registered hotkey's handler sees every hotkey event: only answer our own.
            guard hit.id == key.number else { return OSStatus(eventNotHandledErr) }
            if GetEventKind(event) == UInt32(kEventHotKeyReleased) { key.onRelease?() } else { key.action() }
            return noErr
        }, specs.count, &specs, me, &handler)
        let id = EventHotKeyID(signature: OSType(0x4C4E_5452), id: number)  // "LNTR"
        RegisterEventHotKey(keyCode, modifiers, id, GetApplicationEventTarget(), 0, &ref)
    }

    deinit {
        if let ref { UnregisterEventHotKey(ref) }
        if let handler { RemoveEventHandler(handler) }
    }
}
