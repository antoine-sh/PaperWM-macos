import AppKit
import Carbon.HIToolbox

/// Reports when the user lets go of the switcher's modifiers (⌃⌥, PaperWM's
/// Super, unless rebound), wherever focus is.
@MainActor
final class ModifierWatcher {
    private let onRelease: () -> Void
    private var monitors: [Any] = []
    private var held = false
    /// The modifiers held while switching. Empty when the shortcut has none
    /// but ⇧, so each press switches straight away.
    private var required: NSEvent.ModifierFlags = [.control, .option]

    init(onRelease: @escaping () -> Void) {
        self.onRelease = onRelease
    }

    func start() {
        guard monitors.isEmpty else { return }
        // The global monitor sees other apps' events, the local one ours.
        if let global = NSEvent.addGlobalMonitorForEvents(matching: .flagsChanged, handler: { [weak self] event in
            MainActor.assumeIsolated { self?.update(event.modifierFlags) }
        }) {
            monitors.append(global)
        }
        if let local = NSEvent.addLocalMonitorForEvents(matching: .flagsChanged, handler: { [weak self] event in
            MainActor.assumeIsolated { self?.update(event.modifierFlags) }
            return event
        }) {
            monitors.append(local)
        }
    }

    /// Watches the modifiers of the switcher shortcut, a Carbon mask, less ⇧,
    /// which only picks the backwards variant.
    func watch(carbonModifiers modifiers: Int) {
        required = Self.superFlags(carbonModifiers: modifiers)
    }

    /// The modifiers of a Carbon mask that make up Super: all but ⇧.
    nonisolated static func superFlags(carbonModifiers modifiers: Int) -> NSEvent.ModifierFlags {
        var flags: NSEvent.ModifierFlags = []
        if modifiers & controlKey != 0 { flags.insert(.control) }
        if modifiers & optionKey != 0 { flags.insert(.option) }
        if modifiers & cmdKey != 0 { flags.insert(.command) }
        return flags
    }

    /// Whether the switcher's modifiers are all down right now.
    var superHeld: Bool {
        isHeld(NSEvent.modifierFlags)
    }

    private func isHeld(_ flags: NSEvent.ModifierFlags) -> Bool {
        !required.isEmpty && flags.isSuperset(of: required)
    }

    private func update(_ flags: NSEvent.ModifierFlags) {
        let now = isHeld(flags)
        if held && !now { onRelease() }
        held = now
    }
}
