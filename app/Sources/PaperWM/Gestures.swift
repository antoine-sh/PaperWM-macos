import AppKit
import CPaperWMEngine

/// Trackpad navigation: with ⌃⌥ held, two-finger horizontal scrolling pans
/// the strip and vertical scrolling switches workspaces. macOS keeps
/// three-finger swipes to itself, so this stands in for PaperWM's.
@MainActor
final class Gestures {
    /// Vertical travel, in points, that switches workspace.
    private static let workspaceThreshold = 60.0
    /// Quiet time after which a gesture, momentum included, is over.
    private static let endDelay = 0.15

    private let perform: (Action) -> Void
    private var tap: CFMachPort?
    private var panned = false
    private var vertical = 0.0
    private var switched = false
    private var endTimer: Timer?

    init(perform: @escaping (Action) -> Void) {
        self.perform = perform
    }

    /// Installs the event tap. Needs Accessibility permission.
    func start() {
        guard tap == nil else { return }
        let context = Unmanaged.passUnretained(self).toOpaque()
        let tap = CGEvent.tapCreate(
            tap: .cgSessionEventTap, place: .headInsertEventTap, options: .defaultTap,
            eventsOfInterest: CGEventMask(1 << CGEventType.scrollWheel.rawValue),
            callback: { _, type, event, context in
                guard let context else { return Unmanaged.passUnretained(event) }
                let gestures = Unmanaged<Gestures>.fromOpaque(context).takeUnretainedValue()
                let consumed = MainActor.assumeIsolated { gestures.handle(type, event) }
                return consumed ? nil : Unmanaged.passUnretained(event)
            },
            userInfo: context
        )
        guard let tap else {
            NSLog("PaperWM: could not install the trackpad event tap")
            return
        }
        CFRunLoopAddSource(CFRunLoopGetMain(), CFMachPortCreateRunLoopSource(nil, tap, 0), .commonModes)
        self.tap = tap
    }

    /// Returns whether the event was used and should not reach other apps.
    private func handle(_ type: CGEventType, _ event: CGEvent) -> Bool {
        if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
            if let tap { CGEvent.tapEnable(tap: tap, enable: true) }
            return false
        }
        guard type == .scrollWheel, event.flags.contains([.maskControl, .maskAlternate]) else { return false }

        let dx = event.getDoubleValueField(.scrollWheelEventPointDeltaAxis2)
        let dy = event.getDoubleValueField(.scrollWheelEventPointDeltaAxis1)
        if abs(dx) >= abs(dy) {
            if dx != 0 {
                // The strip follows the fingers.
                perform(.engine(PWM_COMMAND_SCROLL_BY, -dx))
                panned = true
            }
        } else if !switched {
            vertical += dy
            if abs(vertical) >= Self.workspaceThreshold {
                perform(.engine(vertical < 0 ? PWM_COMMAND_NEXT_WORKSPACE : PWM_COMMAND_PREVIOUS_WORKSPACE))
                switched = true
            }
        }

        endTimer?.invalidate()
        endTimer = Timer.scheduledTimer(withTimeInterval: Self.endDelay, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated { self?.finish() }
        }
        return true
    }

    private func finish() {
        if panned { perform(.engine(PWM_COMMAND_SCROLL_END)) }
        panned = false
        vertical = 0
        switched = false
    }
}
