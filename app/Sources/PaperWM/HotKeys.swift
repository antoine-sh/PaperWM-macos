import Carbon.HIToolbox
import CPaperWMEngine

/// A global keyboard shortcut bound to an engine command.
struct HotKey {
    let keyCode: Int
    let modifiers: Int
    let command: PwmCommand
    let title: String

    /// Shortcut as shown in menus, e.g. "⌃⌥←".
    var symbol: String {
        var text = ""
        if modifiers & controlKey != 0 { text += "⌃" }
        if modifiers & optionKey != 0 { text += "⌥" }
        if modifiers & shiftKey != 0 { text += "⇧" }
        if modifiers & cmdKey != 0 { text += "⌘" }
        return text + (HotKey.keyNames[keyCode] ?? "?")
    }

    private static let keyNames: [Int: String] = [
        kVK_LeftArrow: "←", kVK_RightArrow: "→", kVK_UpArrow: "↑", kVK_DownArrow: "↓",
        kVK_ANSI_R: "R", kVK_ANSI_F: "F", kVK_ANSI_C: "C", kVK_ANSI_I: "I", kVK_ANSI_O: "O",
    ]

    /// PaperWM's default bindings, with ⌃⌥ standing in for Super.
    static let defaults: [HotKey] = {
        let base = controlKey | optionKey
        let move = base | shiftKey
        return [
            HotKey(keyCode: kVK_LeftArrow, modifiers: base, command: PWM_COMMAND_FOCUS_LEFT, title: "Focus Left"),
            HotKey(keyCode: kVK_RightArrow, modifiers: base, command: PWM_COMMAND_FOCUS_RIGHT, title: "Focus Right"),
            HotKey(keyCode: kVK_UpArrow, modifiers: base, command: PWM_COMMAND_FOCUS_UP, title: "Focus Up"),
            HotKey(keyCode: kVK_DownArrow, modifiers: base, command: PWM_COMMAND_FOCUS_DOWN, title: "Focus Down"),
            HotKey(keyCode: kVK_LeftArrow, modifiers: move, command: PWM_COMMAND_MOVE_LEFT, title: "Move Column Left"),
            HotKey(keyCode: kVK_RightArrow, modifiers: move, command: PWM_COMMAND_MOVE_RIGHT, title: "Move Column Right"),
            HotKey(keyCode: kVK_UpArrow, modifiers: move, command: PWM_COMMAND_MOVE_UP, title: "Move Window Up"),
            HotKey(keyCode: kVK_DownArrow, modifiers: move, command: PWM_COMMAND_MOVE_DOWN, title: "Move Window Down"),
            HotKey(keyCode: kVK_ANSI_R, modifiers: base, command: PWM_COMMAND_CYCLE_WIDTH, title: "Cycle Column Width"),
            HotKey(keyCode: kVK_ANSI_F, modifiers: base, command: PWM_COMMAND_TOGGLE_FULL_WIDTH, title: "Toggle Full Width"),
            HotKey(keyCode: kVK_ANSI_C, modifiers: base, command: PWM_COMMAND_CENTER, title: "Center Column"),
            HotKey(keyCode: kVK_ANSI_I, modifiers: base, command: PWM_COMMAND_CONSUME, title: "Pull Next Window into Column"),
            HotKey(keyCode: kVK_ANSI_O, modifiers: base, command: PWM_COMMAND_EXPEL, title: "Push Window out of Column"),
        ]
    }()
}

/// Registers global shortcuts with the Carbon hot key API, which needs no
/// extra permission.
@MainActor
final class HotKeyCenter {
    let hotKeys: [HotKey]
    private let action: (PwmCommand) -> Void
    private var registrations: [EventHotKeyRef] = []
    private var handler: EventHandlerRef?

    /// Arbitrary four-character signature identifying our hot keys ("PWM1").
    private static let signature: OSType = 0x5057_4D31

    init(hotKeys: [HotKey], action: @escaping (PwmCommand) -> Void) {
        self.hotKeys = hotKeys
        self.action = action
    }

    func register() {
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

        for (index, hotKey) in hotKeys.enumerated() {
            var ref: EventHotKeyRef?
            let id = EventHotKeyID(signature: HotKeyCenter.signature, id: UInt32(index))
            let status = RegisterEventHotKey(
                UInt32(hotKey.keyCode), UInt32(hotKey.modifiers), id, GetApplicationEventTarget(), 0, &ref
            )
            if status == noErr, let ref {
                registrations.append(ref)
            } else {
                NSLog("PaperWM: could not register \(hotKey.symbol) (\(status)); another app may own it")
            }
        }
    }

    private func fire(_ index: Int) {
        guard hotKeys.indices.contains(index) else { return }
        action(hotKeys[index].command)
    }
}
