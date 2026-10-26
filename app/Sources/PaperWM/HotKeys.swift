import Carbon.HIToolbox
import Foundation
import CPaperWMEngine

/// What a shortcut does.
enum Action: Equatable {
    case engine(PwmCommand, Double = 0)
    /// Opens the window switcher, or steps it forward or back.
    case switcher(forward: Bool)
    /// Flips an on/off setting.
    case toggle(WritableKeyPath<Settings.Values, Bool>)
}

/// A key and Carbon modifier mask (`controlKey | optionKey`, …).
struct KeyCombo: Codable, Equatable {
    var keyCode: Int
    var modifiers: Int

    /// As shown in menus, e.g. "⌃⌥←".
    var symbol: String {
        var text = ""
        if modifiers & controlKey != 0 { text += "⌃" }
        if modifiers & optionKey != 0 { text += "⌥" }
        if modifiers & shiftKey != 0 { text += "⇧" }
        if modifiers & cmdKey != 0 { text += "⌘" }
        return text + (KeyCombo.keyNames[keyCode] ?? "#\(keyCode)")
    }

    private static let keyNames: [Int: String] = {
        var names: [Int: String] = [
            kVK_LeftArrow: "←", kVK_RightArrow: "→", kVK_UpArrow: "↑", kVK_DownArrow: "↓",
            kVK_PageUp: "⇞", kVK_PageDown: "⇟", kVK_Home: "↖", kVK_End: "↘",
            kVK_Escape: "⎋", kVK_Tab: "⇥", kVK_Return: "↩", kVK_Space: "Space", kVK_Delete: "⌫",
            kVK_ANSI_Grave: "`", kVK_ANSI_Minus: "-", kVK_ANSI_Equal: "=", kVK_ANSI_LeftBracket: "[",
            kVK_ANSI_RightBracket: "]", kVK_ANSI_Semicolon: ";", kVK_ANSI_Quote: "'", kVK_ANSI_Comma: ",",
            kVK_ANSI_Period: ".", kVK_ANSI_Slash: "/", kVK_ANSI_Backslash: "\\",
        ]
        let letters = [
            kVK_ANSI_A, kVK_ANSI_B, kVK_ANSI_C, kVK_ANSI_D, kVK_ANSI_E, kVK_ANSI_F, kVK_ANSI_G, kVK_ANSI_H,
            kVK_ANSI_I, kVK_ANSI_J, kVK_ANSI_K, kVK_ANSI_L, kVK_ANSI_M, kVK_ANSI_N, kVK_ANSI_O, kVK_ANSI_P,
            kVK_ANSI_Q, kVK_ANSI_R, kVK_ANSI_S, kVK_ANSI_T, kVK_ANSI_U, kVK_ANSI_V, kVK_ANSI_W, kVK_ANSI_X,
            kVK_ANSI_Y, kVK_ANSI_Z,
        ]
        for (i, code) in letters.enumerated() {
            names[code] = String(UnicodeScalar(UInt8(65 + i)))
        }
        for (i, code) in HotKey.digitKeys.enumerated() {
            names[code] = String(i)
        }
        let functionKeys = [
            kVK_F1, kVK_F2, kVK_F3, kVK_F4, kVK_F5, kVK_F6, kVK_F7, kVK_F8, kVK_F9, kVK_F10, kVK_F11, kVK_F12,
        ]
        for (i, code) in functionKeys.enumerated() {
            names[code] = "F\(i + 1)"
        }
        return names
    }()
}

/// A global keyboard shortcut.
struct HotKey {
    /// Groups shortcuts in Settings and the README, in display order.
    enum Category: String, CaseIterable {
        case focus = "Focus and Move"
        case workspaces = "Workspaces"
        case monitors = "Monitors"
        case size = "Size"
        case layout = "Column Layout"
        case switcher = "Switcher and Scratch Layer"
        case appearance = "Appearance"
    }

    /// Stable name used to store the user's override, e.g. "focus-left".
    let id: String
    let title: String
    let category: Category
    let action: Action
    var combo: KeyCombo

    static let digitKeys = [
        kVK_ANSI_0, kVK_ANSI_1, kVK_ANSI_2, kVK_ANSI_3, kVK_ANSI_4,
        kVK_ANSI_5, kVK_ANSI_6, kVK_ANSI_7, kVK_ANSI_8, kVK_ANSI_9,
    ]

    /// PaperWM's bindings adapted to Mac keyboards, with ⌃⌥ standing in for
    /// Super. Within a category a key focuses or switches and adding ⇧ takes
    /// the window along (or gives the opposite or vertical variant). Every
    /// key exists on laptop keyboards without Fn, and no shortcut needs more
    /// than three modifiers.
    static let defaults: [HotKey] = {
        let base = controlKey | optionKey
        let shift = base | shiftKey
        // id, title, key code, modifiers, action.
        typealias Entry = (String, String, Int, Int, Action)
        var all: [HotKey] = []
        func add(_ category: Category, _ entries: [Entry]) {
            for (id, title, keyCode, modifiers, action) in entries {
                all.append(HotKey(id: id, title: title, category: category, action: action,
                                  combo: KeyCombo(keyCode: keyCode, modifiers: modifiers)))
            }
        }
        let workspaces = 1...Int(PWM_WORKSPACE_COUNT)
        add(.focus, [
            ("focus-left", "Focus Left", kVK_LeftArrow, base, .engine(PWM_COMMAND_FOCUS_LEFT)),
            ("focus-right", "Focus Right", kVK_RightArrow, base, .engine(PWM_COMMAND_FOCUS_RIGHT)),
            ("focus-up", "Focus Up", kVK_UpArrow, base, .engine(PWM_COMMAND_FOCUS_UP)),
            ("focus-down", "Focus Down", kVK_DownArrow, base, .engine(PWM_COMMAND_FOCUS_DOWN)),
            // ⌘ for "all the way", as ⌘← jumps to the start of a line.
            ("focus-first", "Focus First Column", kVK_LeftArrow, base | cmdKey, .engine(PWM_COMMAND_FOCUS_FIRST)),
            ("focus-last", "Focus Last Column", kVK_RightArrow, base | cmdKey, .engine(PWM_COMMAND_FOCUS_LAST)),
            ("move-left", "Move Column Left", kVK_LeftArrow, shift, .engine(PWM_COMMAND_MOVE_LEFT)),
            ("move-right", "Move Column Right", kVK_RightArrow, shift, .engine(PWM_COMMAND_MOVE_RIGHT)),
            ("move-up", "Move Window Up", kVK_UpArrow, shift, .engine(PWM_COMMAND_MOVE_UP)),
            ("move-down", "Move Window Down", kVK_DownArrow, shift, .engine(PWM_COMMAND_MOVE_DOWN)),
        ])
        add(.workspaces, workspaces.map { n -> Entry in
            ("switch-workspace-\(n)", "Switch to Workspace \(n)", digitKeys[n], base,
             .engine(PWM_COMMAND_SWITCH_WORKSPACE, Double(n - 1)))
        })
        add(.workspaces, workspaces.map { n -> Entry in
            ("move-to-workspace-\(n)", "Move Window to Workspace \(n)", digitKeys[n], shift,
             .engine(PWM_COMMAND_MOVE_TO_WORKSPACE, Double(n - 1)))
        })
        add(.workspaces, [
            ("previous-workspace", "Previous Workspace", kVK_ANSI_LeftBracket, base,
             .engine(PWM_COMMAND_PREVIOUS_WORKSPACE)),
            ("next-workspace", "Next Workspace", kVK_ANSI_RightBracket, base, .engine(PWM_COMMAND_NEXT_WORKSPACE)),
            ("move-to-previous-workspace", "Move Window to Previous Workspace", kVK_ANSI_LeftBracket, shift,
             .engine(PWM_COMMAND_MOVE_TO_PREVIOUS_WORKSPACE)),
            ("move-to-next-workspace", "Move Window to Next Workspace", kVK_ANSI_RightBracket, shift,
             .engine(PWM_COMMAND_MOVE_TO_NEXT_WORKSPACE)),
            ("last-workspace", "Last Used Workspace", kVK_ANSI_Grave, base, .engine(PWM_COMMAND_LAST_WORKSPACE)),
        ])
        // , and . carry < and >.
        add(.monitors, [
            ("focus-monitor-left", "Focus Monitor Left", kVK_ANSI_Comma, base,
             .engine(PWM_COMMAND_FOCUS_MONITOR_LEFT)),
            ("focus-monitor-right", "Focus Monitor Right", kVK_ANSI_Period, base,
             .engine(PWM_COMMAND_FOCUS_MONITOR_RIGHT)),
            ("move-to-monitor-left", "Move Window to Monitor Left", kVK_ANSI_Comma, shift,
             .engine(PWM_COMMAND_MOVE_TO_MONITOR_LEFT)),
            ("move-to-monitor-right", "Move Window to Monitor Right", kVK_ANSI_Period, shift,
             .engine(PWM_COMMAND_MOVE_TO_MONITOR_RIGHT)),
        ])
        add(.size, [
            ("cycle-width", "Cycle Column Width", kVK_ANSI_R, base, .engine(PWM_COMMAND_CYCLE_WIDTH)),
            ("full-width", "Toggle Full Width", kVK_ANSI_F, base, .engine(PWM_COMMAND_TOGGLE_FULL_WIDTH)),
            ("widen-column", "Widen Column", kVK_ANSI_Equal, base, .engine(PWM_COMMAND_WIDEN_COLUMN)),
            ("narrow-column", "Narrow Column", kVK_ANSI_Minus, base, .engine(PWM_COMMAND_NARROW_COLUMN)),
            ("taller", "Make Window Taller", kVK_ANSI_Equal, shift, .engine(PWM_COMMAND_TALLER)),
            ("shorter", "Make Window Shorter", kVK_ANSI_Minus, shift, .engine(PWM_COMMAND_SHORTER)),
            ("equal-heights", "Equalize Window Heights", kVK_ANSI_E, base, .engine(PWM_COMMAND_EQUAL_HEIGHTS)),
        ])
        add(.layout, [
            ("center", "Center Column", kVK_ANSI_C, base, .engine(PWM_COMMAND_CENTER)),
            ("consume", "Pull Next Window into Column", kVK_ANSI_I, base, .engine(PWM_COMMAND_CONSUME)),
            ("expel", "Push Window out of Column", kVK_ANSI_O, base, .engine(PWM_COMMAND_EXPEL)),
            ("toggle-tabbed", "Toggle Tabbed Column", kVK_ANSI_T, base, .engine(PWM_COMMAND_TOGGLE_TABBED)),
            ("pin-left", "Pin Column to Left Edge", kVK_ANSI_P, base, .engine(PWM_COMMAND_TOGGLE_PIN_LEFT)),
            ("pin-right", "Pin Column to Right Edge", kVK_ANSI_P, shift, .engine(PWM_COMMAND_TOGGLE_PIN_RIGHT)),
        ])
        add(.switcher, [
            ("switcher", "Switch Windows", kVK_Tab, base, .switcher(forward: true)),
            ("switcher-back", "Switch Windows Backwards", kVK_Tab, shift, .switcher(forward: false)),
            ("toggle-scratch", "Show or Hide Scratch Layer", kVK_Escape, base, .engine(PWM_COMMAND_TOGGLE_SCRATCH)),
            ("move-to-scratch", "Move Window to or from Scratch Layer", kVK_Escape, shift,
             .engine(PWM_COMMAND_MOVE_TO_SCRATCH)),
        ])
        add(.appearance, [
            ("toggle-border", "Show or Hide Focus Border", kVK_ANSI_B, base, .toggle(\.focusBorder)),
            ("toggle-dim", "Dim Background Windows", kVK_ANSI_D, base, .toggle(\.dimBackground)),
        ])
        return all
    }()
}

/// Registers global shortcuts with the Carbon hot key API, which needs no
/// extra permission.
@MainActor
final class HotKeyCenter {
    private(set) var hotKeys: [HotKey] = []
    /// Ids of shortcuts macOS refused, usually because another app owns them.
    private(set) var failed: Set<String> = []
    private let action: (Action) -> Void
    private var registrations: [EventHotKeyRef] = []
    private var handler: EventHandlerRef?

    /// Arbitrary four-character signature identifying our hot keys ("PWM1").
    private static let signature: OSType = 0x5057_4D31

    init(action: @escaping (Action) -> Void) {
        self.action = action
    }

    /// Replaces the registered shortcuts with `hotKeys`.
    func register(_ hotKeys: [HotKey]) {
        installHandler()
        unregisterAll()
        self.hotKeys = hotKeys
        failed = []
        for (index, hotKey) in hotKeys.enumerated() {
            var ref: EventHotKeyRef?
            let id = EventHotKeyID(signature: HotKeyCenter.signature, id: UInt32(index))
            let status = RegisterEventHotKey(
                UInt32(hotKey.combo.keyCode), UInt32(hotKey.combo.modifiers), id, GetApplicationEventTarget(), 0, &ref
            )
            if status == noErr, let ref {
                registrations.append(ref)
            } else {
                failed.insert(hotKey.id)
                NSLog("PaperWM: could not register \(hotKey.combo.symbol) (\(status)); another app may own it")
            }
        }
    }

    /// Releases every shortcut, e.g. so a shortcut recorder sees the keys.
    func unregisterAll() {
        for ref in registrations {
            UnregisterEventHotKey(ref)
        }
        registrations = []
    }

    private func installHandler() {
        guard handler == nil else { return }
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        let context = Unmanaged.passUnretained(self).toOpaque()
        InstallEventHandler(GetApplicationEventTarget(), { _, event, context in
            guard let event, let context else { return OSStatus(eventNotHandledErr) }
            var id = EventHotKeyID()
            let status = GetEventParameter(
                event, EventParamName(kEventParamDirectObject), EventParamType(typeEventHotKeyID),
                nil, MemoryLayout<EventHotKeyID>.size, nil, &id
            )
            guard status == noErr, id.signature == HotKeyCenter.signature else { return OSStatus(eventNotHandledErr) }
            let center = Unmanaged<HotKeyCenter>.fromOpaque(context).takeUnretainedValue()
            MainActor.assumeIsolated { center.fire(Int(id.id)) }
            return noErr
        }, 1, &spec, context, &handler)
    }

    private func fire(_ index: Int) {
        guard hotKeys.indices.contains(index) else { return }
        action(hotKeys[index].action)
    }
}
