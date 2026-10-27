import AppKit
import CPaperWMEngine

/// Trackpad navigation: with Super held (⌃⌥, or the switcher shortcut's
/// modifiers), two-finger horizontal scrolling pans the strip and vertical
/// scrolling switches workspaces. macOS keeps three-finger swipes to itself,
/// so this stands in for PaperWM's.
@MainActor
final class Gestures {
    /// Vertical travel, in points, that switches workspace.
    private static let workspaceThreshold = 60.0
    /// Quiet time after which a gesture, momentum included, is over.
    private static let endDelay = 0.15

    private let perform: (Action) -> Void
    private var tap: ScrollTap?
    private var panned = false
    private var vertical = 0.0
    private var switched = false
    private var endTimer: Timer?
    /// Modifiers that must be held, kept until the tap exists.
    private var required: NSEvent.ModifierFlags = [.control, .option]

    init(perform: @escaping (Action) -> Void) {
        self.perform = perform
    }

    /// Installs the event tap. Needs Accessibility permission.
    func start() {
        guard tap == nil else { return }
        tap = ScrollTap { [weak self] dx, dy in
            MainActor.assumeIsolated { self?.handle(dx: dx, dy: dy) }
        }
        tap?.setRequired(required)
        if tap?.start() != true {
            tap = nil
            NSLog("PaperWM: could not install the trackpad event tap")
        }
    }

    /// Pans only while the modifiers of the Carbon mask `modifiers`, less ⇧,
    /// are held; with none left, scrolling is never taken over.
    func watch(carbonModifiers modifiers: Int) {
        required = ModifierWatcher.superFlags(carbonModifiers: modifiers)
        tap?.setRequired(required)
    }

    /// Handles the scrolling gathered since the last call.
    private func handle(dx: Double, dy: Double) {
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

        let fireDate = Date(timeIntervalSinceNow: Self.endDelay)
        if let endTimer, endTimer.isValid {
            endTimer.fireDate = fireDate
        } else {
            let timer = Timer(fire: fireDate, interval: 0, repeats: false) { [weak self] _ in
                MainActor.assumeIsolated { self?.finish() }
            }
            RunLoop.main.add(timer, forMode: .common)
            endTimer = timer
        }
    }

    private func finish() {
        endTimer = nil
        if panned { perform(.engine(PWM_COMMAND_SCROLL_END)) }
        panned = false
        vertical = 0
        switched = false
    }
}

/// The scroll event tap, on a thread of its own. An active tap holds every
/// scroll event in every app until its callback returns, so the callback
/// must never wait for the main thread, which spends time in slow
/// Accessibility calls. It only decides whether to swallow the event and
/// adds the deltas up; the main thread takes the sum whenever it is free,
/// which also coalesces events that arrive while it is busy.
private final class ScrollTap: @unchecked Sendable {
    private let deliver: @Sendable (Double, Double) -> Void
    private let lock = NSLock()
    private var dx = 0.0
    private var dy = 0.0
    private var deliveryScheduled = false
    /// Guarded by `lock`: the main thread sets it, the tap thread reads it.
    private var required: CGEventFlags = [.maskControl, .maskAlternate]
    /// Set once in `start`, before the tap thread runs.
    private var port: CFMachPort?

    init(deliver: @escaping @Sendable (Double, Double) -> Void) {
        self.deliver = deliver
    }

    func start() -> Bool {
        let context = Unmanaged.passUnretained(self).toOpaque()
        guard let port = CGEvent.tapCreate(
            tap: .cgSessionEventTap, place: .headInsertEventTap, options: .defaultTap,
            eventsOfInterest: CGEventMask(1 << CGEventType.scrollWheel.rawValue),
            callback: { _, type, event, context in
                guard let context else { return Unmanaged.passUnretained(event) }
                let tap = Unmanaged<ScrollTap>.fromOpaque(context).takeUnretainedValue()
                return tap.handle(type, event) ? nil : Unmanaged.passUnretained(event)
            },
            userInfo: context
        ) else { return false }
        self.port = port
        let source = CFMachPortCreateRunLoopSource(nil, port, 0)
        let thread = Thread {
            CFRunLoopAddSource(CFRunLoopGetCurrent(), source, .commonModes)
            CGEvent.tapEnable(tap: port, enable: true)
            CFRunLoopRun()
        }
        thread.name = "PaperWM scroll tap"
        thread.qualityOfService = .userInteractive
        thread.start()
        return true
    }

    /// Runs on the tap thread. Returns whether to swallow the event.
    private func handle(_ type: CGEventType, _ event: CGEvent) -> Bool {
        if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
            if let port { CGEvent.tapEnable(tap: port, enable: true) }
            return false
        }
        guard type == .scrollWheel else { return false }

        lock.lock()
        guard !required.isEmpty, event.flags.contains(required) else {
            lock.unlock()
            return false
        }
        dx += event.getDoubleValueField(.scrollWheelEventPointDeltaAxis2)
        dy += event.getDoubleValueField(.scrollWheelEventPointDeltaAxis1)
        let schedule = !deliveryScheduled
        deliveryScheduled = true
        lock.unlock()
        if schedule {
            DispatchQueue.main.async { self.flush() }
        }
        return true
    }

    func setRequired(_ flags: NSEvent.ModifierFlags) {
        // Device-independent modifier bits are the same in both types.
        let required = CGEventFlags(rawValue: UInt64(flags.rawValue))
        lock.lock()
        self.required = required
        lock.unlock()
    }

    /// Runs on the main thread: hands over everything gathered so far.
    private func flush() {
        lock.lock()
        let (dx, dy) = (self.dx, self.dy)
        self.dx = 0
        self.dy = 0
        deliveryScheduled = false
        lock.unlock()
        deliver(dx, dy)
    }
}
