import AppKit

/// Reports when the user lets go of ⌃⌥ (PaperWM's Super), wherever focus is.
@MainActor
final class ModifierWatcher {
    private let onRelease: () -> Void
    private var monitors: [Any] = []
    private var held = false

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

    /// Whether ⌃ and ⌥ are both down right now.
    static var superHeld: Bool {
        NSEvent.modifierFlags.isSuperset(of: [.control, .option])
    }

    private func update(_ flags: NSEvent.ModifierFlags) {
        let now = flags.isSuperset(of: [.control, .option])
        if held && !now { onRelease() }
        held = now
    }
}
