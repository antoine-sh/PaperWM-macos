import AppKit
import CPaperWMEngine

/// Trackpad navigation: with Super held (⌃⌥, or the switcher shortcut's
/// modifiers), two-finger horizontal scrolling pans the strip. macOS keeps
/// three-finger swipes to itself, so this stands in for PaperWM's. Optionally,
/// plain three- or four-finger swipes pan it too, once the matching macOS
/// swipe gestures are turned off in System Settings › Trackpad.
@MainActor
final class Gestures {
    /// Quiet time after which a gesture, momentum included, is over.
    private static let endDelay = 0.15

    private let perform: (Action) -> Void
    private var tap: ScrollTap?
    private var panned = false
    private var endTimer: Timer?
    /// Modifiers that must be held, kept until the tap exists.
    private var required: NSEvent.ModifierFlags = [.control, .option]
    private var swipeTap: CFMachPort?
    /// Mean finger position on the trackpad (0–1) at the last swipe event;
    /// nil while no swipe with the right number of fingers is under way.
    private var swipeLast: CGPoint?
    /// Movement of a swipe before its direction is settled.
    private var swipeTravel = CGPoint.zero
    /// nil until the direction is settled; vertical swipes are left alone.
    private var swipeHorizontal: Bool?
    private var swiped = false

    /// Swipes with this many fingers pan the strip with no modifier held; 0 turns them off.
    var swipeFingers = 0 {
        didSet {
            endSwipe()
            if swipeFingers > 0 { startSwipeTap() }
        }
    }

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
        if abs(dx) >= abs(dy), dx != 0 {
            // The strip follows the fingers.
            perform(.engine(PWM_COMMAND_SCROLL_BY, -dx))
            panned = true
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

    /// Listens to the trackpad's touch events. Listen-only, so it never holds
    /// events back and can run on the main thread; macOS can't be stopped
    /// from acting on the same swipe, hence turning its gesture off.
    private func startSwipeTap() {
        guard swipeTap == nil else { return }
        let context = Unmanaged.passUnretained(self).toOpaque()
        guard let port = CGEvent.tapCreate(
            tap: .cgSessionEventTap, place: .headInsertEventTap, options: .listenOnly,
            eventsOfInterest: CGEventMask(1) << gestureEventType,
            callback: { _, type, event, context in
                guard let context else { return Unmanaged.passUnretained(event) }
                MainActor.assumeIsolated {
                    Unmanaged<Gestures>.fromOpaque(context).takeUnretainedValue().swipe(type, event)
                }
                return Unmanaged.passUnretained(event)
            },
            userInfo: context
        ) else {
            NSLog("PaperWM: could not install the trackpad swipe tap")
            return
        }
        swipeTap = port
        CFRunLoopAddSource(CFRunLoopGetMain(), CFMachPortCreateRunLoopSource(nil, port, 0), .commonModes)
        CGEvent.tapEnable(tap: port, enable: true)
    }

    private func swipe(_ type: CGEventType, _ event: CGEvent) {
        if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
            if let swipeTap { CGEvent.tapEnable(tap: swipeTap, enable: true) }
            return
        }
        guard swipeFingers > 0, type.rawValue == gestureEventType, let event = NSEvent(cgEvent: event) else {
            return
        }
        let touches = event.touches(matching: .touching, in: nil)
        guard touches.count == swipeFingers else { return endSwipe() }
        let sum = touches.reduce(CGPoint.zero) {
            CGPoint(x: $0.x + $1.normalizedPosition.x, y: $0.y + $1.normalizedPosition.y)
        }
        let position = CGPoint(x: sum.x / CGFloat(touches.count), y: sum.y / CGFloat(touches.count))
        defer { swipeLast = position }
        guard let last = swipeLast else { return }
        let dx = position.x - last.x
        if swipeHorizontal == nil {
            swipeTravel.x += dx
            swipeTravel.y += position.y - last.y
            // A few percent of the trackpad tells a sideways swipe from Mission Control's.
            guard max(abs(swipeTravel.x), abs(swipeTravel.y)) > 0.03 else { return }
            swipeHorizontal = abs(swipeTravel.x) > abs(swipeTravel.y)
        }
        guard swipeHorizontal == true, dx != 0, let width = NSScreen.main?.frame.width else { return }
        // The strip follows the fingers: the trackpad's width pans one screen.
        perform(.engine(PWM_COMMAND_SCROLL_BY, -Double(dx * width)))
        swiped = true
    }

    private func endSwipe() {
        if swiped { perform(.engine(PWM_COMMAND_SCROLL_END)) }
        swiped = false
        swipeLast = nil
        swipeTravel = .zero
        swipeHorizontal = nil
    }

    private func finish() {
        endTimer = nil
        if panned { perform(.engine(PWM_COMMAND_SCROLL_END)) }
        panned = false
    }
}

/// `NSEvent.EventType.gesture`, which `CGEventType` has no case for.
private let gestureEventType: UInt32 = 29

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
