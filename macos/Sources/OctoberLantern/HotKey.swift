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

/// A global hotkey through Carbon's RegisterEventHotKey, which needs no Accessibility permission.
final class HotKey {
    private var ref: EventHotKeyRef?
    private var handler: EventHandlerRef?
    private let action: () -> Void

    /// Defaults to ⌃⌥Space.
    init(keyCode: UInt32 = UInt32(kVK_Space), modifiers: UInt32 = UInt32(controlKey | optionKey), action: @escaping () -> Void) {
        self.action = action
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        let me = Unmanaged.passUnretained(self).toOpaque()
        InstallEventHandler(GetApplicationEventTarget(), { _, _, userData in
            guard let userData else { return noErr }
            Unmanaged<HotKey>.fromOpaque(userData).takeUnretainedValue().action()
            return noErr
        }, 1, &spec, me, &handler)
        let id = EventHotKeyID(signature: OSType(0x4C4E_5452), id: 1)  // "LNTR"
        RegisterEventHotKey(keyCode, modifiers, id, GetApplicationEventTarget(), 0, &ref)
    }

    deinit {
        if let ref { UnregisterEventHotKey(ref) }
        if let handler { RemoveEventHandler(handler) }
    }
}
