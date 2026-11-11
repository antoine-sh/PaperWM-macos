import AppKit
import CPaperWMEngine
import os

/// Connects the engine to the system: tells it about monitors and apps,
/// syncs it when windows change, and routes hot keys to it.
@MainActor
final class WindowManager {
    private(set) var engine = Engine()
    let settings = Settings()
    private(set) lazy var hotKeys = HotKeyCenter { [weak self] action in
        self?.perform(action)
    }
    private let minimap = Minimap()
    private let switcher = Switcher()
    private let decorations = Decorations()
    private lazy var tabBars = TabBars { [weak self] id in
        self?.perform(.engine(PWM_COMMAND_FOCUS_WINDOW, Double(id)))
    }
    private lazy var gestures = Gestures { [weak self] action in self?.perform(action) }
    private lazy var modifiers = ModifierWatcher { [weak self] in self?.superReleased() }
    private lazy var drags = DragWatcher { [weak self] point in self?.dropped(at: point) }
    private var syncScheduled = false
    /// Whether a sync is due because the last one met a window still being created.
    private var settleScheduled = false
    /// A sync asked for during a slide or a drag, run when it ends.
    private var syncDeferred = false
    private var hotKeysPaused = false
    /// Off leaves every window where the user puts it; on tiles afresh.
    private(set) var isEnabled = true
    /// Called with each layout published, e.g. to keep the menu bar panel current.
    var onPublish: ((EngineState) -> Void)?
    /// What was last applied, so settings edits only redo what changed.
    private var appliedConfig = ""
    private var appliedShortcuts: [KeyCombo] = []
    /// Drives strip slides while one is running, once per display frame.
    private lazy var frames = FrameDriver { [weak self] in self?.step() }
    private var lastTick: CFTimeInterval = 0
    /// Why windows joined or left the layout, and the sleep, wake and Space
    /// changes around it: `log stream --predicate 'subsystem == "io.github.antoine-sh.PaperWM"'`.
    private let log = Logger(subsystem: "io.github.antoine-sh.PaperWM", category: "windows")
    /// Bumped by each sync, so only the latest scheduled recheck runs.
    private var recheckGeneration = 0
    /// The least wait before a recheck, doubled while a window stays due
    /// (its app busy, or syncs failing) so the rechecks back off.
    private var recheckFloor = 0.5
    /// When each distinct error was last logged, to keep repeats out of the log.
    private var lastLogged: [String: CFTimeInterval] = [:]
    /// Each wake notification, with the sleep notification it ends. Windows
    /// leave the screen meanwhile, which the engine must take for away.
    private static let wakes: [Notification.Name: Notification.Name] = [
        NSWorkspace.didWakeNotification: NSWorkspace.willSleepNotification,
        NSWorkspace.screensDidWakeNotification: NSWorkspace.screensDidSleepNotification,
        NSWorkspace.sessionDidBecomeActiveNotification: NSWorkspace.sessionDidResignActiveNotification,
        Notification.Name("com.apple.screenIsUnlocked"): Notification.Name("com.apple.screenIsLocked"),
    ]

    func start() {
        connectEngine()
        let workspace = NSWorkspace.shared.notificationCenter
        workspace.addObserver(self, selector: #selector(appLaunched(_:)),
                              name: NSWorkspace.didLaunchApplicationNotification, object: nil)
        workspace.addObserver(self, selector: #selector(appTerminated(_:)),
                              name: NSWorkspace.didTerminateApplicationNotification, object: nil)
        workspace.addObserver(self, selector: #selector(appActivated(_:)),
                              name: NSWorkspace.didActivateApplicationNotification, object: nil)
        for name in [NSWorkspace.activeSpaceDidChangeNotification, NSWorkspace.didUnhideApplicationNotification] {
            workspace.addObserver(self, selector: #selector(somethingChanged(_:)), name: name, object: nil)
        }
        workspace.addObserver(self, selector: #selector(logOnly(_:)),
                              name: NSWorkspace.didHideApplicationNotification, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(screensChanged(_:)),
                                               name: NSApplication.didChangeScreenParametersNotification, object: nil)
        let locks = DistributedNotificationCenter.default()
        for (wake, sleep) in Self.wakes {
            if wake.rawValue.hasPrefix("com.apple.") {
                // A menu bar app is rarely active, and distributed
                // notifications wait for that by default.
                locks.addObserver(self, selector: #selector(slept(_:)), name: sleep, object: nil,
                                  suspensionBehavior: .deliverImmediately)
                locks.addObserver(self, selector: #selector(woke(_:)), name: wake, object: nil,
                                  suspensionBehavior: .deliverImmediately)
            } else {
                workspace.addObserver(self, selector: #selector(slept(_:)), name: sleep, object: nil)
                workspace.addObserver(self, selector: #selector(woke(_:)), name: wake, object: nil)
            }
        }

        settings.onChange = { [weak self] in self?.applySettings() }
        applySettings()
        updateMonitors()
        modifiers.start()
        drags.start()
        gestures.start()
    }

    /// Hooks the engine up to change notifications and every running app.
    private func connectEngine() {
        let context = Unmanaged.passUnretained(self).toOpaque()
        engine.setChangeHandler({ context in
            guard let context else { return }
            let manager = Unmanaged<WindowManager>.fromOpaque(context).takeUnretainedValue()
            MainActor.assumeIsolated { manager.scheduleSync() }
        }, context: context)
        for app in NSWorkspace.shared.runningApplications where app.activationPolicy == .regular {
            engine.observeApp(app.processIdentifier)
        }
    }

    /// Pauses global shortcuts, e.g. while one is being recorded.
    func setHotKeysPaused(_ paused: Bool) {
        hotKeysPaused = paused
        if paused || !isEnabled {
            hotKeys.unregisterAll()
        } else {
            hotKeys.register(settings.hotKeys)
        }
    }

    /// Turns tiling off, bringing every window back on screen, or on again
    /// from scratch, since windows were moved freely meanwhile.
    func setEnabled(_ enabled: Bool) {
        guard enabled != isEnabled else { return }
        isEnabled = enabled
        setHotKeysPaused(hotKeysPaused)
        if enabled {
            // Watch the switcher's modifiers again; `retile` reapplies the rest.
            appliedShortcuts = []
            retile()
            return
        }
        // Lets ⌃⌥-scrolling through to apps.
        gestures.watch(carbonModifiers: 0)
        frames.stop()
        syncDeferred = false
        if switcher.isOpen { switcher.cancel() }
        minimap.hide()
        tabBars.hide()
        decorations.hide()
        report { try engine.release() }
    }

    /// Applies a layout from the menu bar panel to the active strip.
    func arrange(_ layout: [(rows: Int, fraction: Double)]) {
        guard isEnabled else { return }
        report { try engine.arrange(layout) }
        publish()
    }

    func perform(_ action: Action) {
        if case .toggle(let setting) = action {
            settings.values[keyPath: setting].toggle()
            return
        }
        guard isEnabled else { return }
        switch action {
        case .engine(PWM_COMMAND_TOGGLE_SCRATCH, _) where switcher.isOpen:
            // ⌃⌥Esc while switching cancels, as Esc does in PaperWM.
            switcher.cancel()
        case .engine(PWM_COMMAND_SCROLL_BY, let arg):
            // Pans arrive every frame: move the windows and the decorations,
            // and leave the full update to the end of the pan.
            report { try engine.run(PWM_COMMAND_SCROLL_BY, arg: arg) }
            decorations.track(placed: engine.placedFrame)
        case .engine(let command, let arg):
            var changed = false
            report { changed = try engine.run(command, arg: arg) }
            let state = publish()
            // Focusing a picked window is not navigating the strip.
            if let state, changed || command == PWM_COMMAND_SCROLL_END, command != PWM_COMMAND_FOCUS_WINDOW {
                minimap.show(state, style: settings.values.minimapStyle)
            }
        case .switcher(let forward):
            guard let state = engine.state() else { return }
            minimap.hide()
            switcher.step(forward: forward, state: state)
            // A quick tap that is already over switches straight away.
            if !modifiers.superHeld { superReleased() }
        case .toggle:
            break
        }
    }

    private func superReleased() {
        minimap.hide()
        if let id = switcher.commit() {
            perform(.engine(PWM_COMMAND_FOCUS_WINDOW, Double(id)))
        }
    }

    private func applySettings() {
        // Changing the gap or app rules would tile again; turning tiling on applies everything afresh.
        guard isEnabled else { return }
        let config = settings.engineConfigJSON
        if config != appliedConfig {
            appliedConfig = config
            report { try engine.setConfig(json: config) }
            publish()
        }
        let shortcuts = settings.hotKeys.map(\.combo)
        if shortcuts != appliedShortcuts {
            appliedShortcuts = shortcuts
            if !hotKeysPaused { hotKeys.register(settings.hotKeys) }
            if let binding = settings.hotKeys.first(where: { $0.id == "switcher" }) {
                modifiers.watch(carbonModifiers: binding.combo.modifiers)
                gestures.watch(carbonModifiers: binding.combo.modifiers)
            }
        }
        decorations.apply(settings.values)
    }

    @discardableResult
    private func publish() -> EngineState? {
        guard isEnabled else { return nil }
        animate()
        settle()
        recheck()
        guard let state = engine.state() else {
            decorations.refresh(focus: nil)
            return nil
        }
        // A dragged window is under the pointer, not where the engine put it.
        decorations.refresh(focus: state.focusedWindow) { drags.isDragging ? nil : engine.placedFrame($0) }
        tabBars.update(state)
        onPublish?(state)
        return state
    }

    /// Syncs again shortly while a new window is still being created:
    /// macOS sends nothing once it is on screen and tileable.
    private func settle() {
        guard !settleScheduled, engine.isSettling else { return }
        settleScheduled = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { [weak self] in
            MainActor.assumeIsolated {
                self?.settleScheduled = false
                self?.scheduleSync()
            }
        }
    }

    /// Syncs once more when a missing window may count as closed, since
    /// nothing notifies then.
    private func recheck() {
        recheckGeneration += 1
        guard let delay = engine.nextCheck else {
            recheckFloor = 0.5
            return
        }
        let wait = max(delay + 0.05, recheckFloor)
        recheckFloor = delay < 0.05 ? min(recheckFloor * 2, 8) : 0.5
        let generation = recheckGeneration
        DispatchQueue.main.asyncAfter(deadline: .now() + wait) { [weak self] in
            MainActor.assumeIsolated {
                guard let self, self.recheckGeneration == generation else { return }
                self.scheduleSync()
            }
        }
    }

    /// Ticks the engine every frame until the strips stop sliding.
    private func animate() {
        guard !frames.isRunning, engine.isAnimating else { return }
        lastTick = CACurrentMediaTime()
        frames.start()
    }

    private func step() {
        // Real elapsed time, so slow Accessibility calls skip frames instead
        // of stretching the slide.
        let now = CACurrentMediaTime()
        var sliding = false
        report { sliding = try engine.tick(now - lastTick) }
        lastTick = now
        decorations.track(placed: engine.placedFrame)
        guard !sliding else { return }
        frames.stop()
        if syncDeferred && !drags.isDragging {
            syncDeferred = false
            syncNow()
        }
    }

    /// Syncs shortly, coalescing bursts of notifications: one focus change
    /// sends several, a few milliseconds apart.
    func scheduleSync() {
        guard isEnabled, !syncScheduled else { return }
        syncScheduled = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.03) { [weak self] in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.syncScheduled = false
                // Reading windows mid-slide or mid-drag only finds them in
                // motion, and would put a dragged window back under the pointer.
                if self.frames.isRunning || self.drags.isDragging {
                    self.syncDeferred = true
                } else {
                    self.syncNow()
                }
            }
        }
    }

    func syncNow() {
        guard isEnabled else { return }
        report { try engine.sync() }
        publish()
    }

    /// Moves a window dragged onto another column there; the drop syncs in
    /// place of any sync held back during the drag.
    private func dropped(at point: CGPoint) {
        guard isEnabled else { return }
        syncDeferred = false
        report { try engine.drop(at: point) }
        publish()
    }

    /// Restarts the engine from scratch: forgets all layout and observers,
    /// then re-reads every app, window and monitor as on launch.
    func retile() {
        frames.stop()
        syncDeferred = false
        engine = Engine()
        connectEngine()
        appliedConfig = ""
        applySettings()
        updateMonitors()
    }

    private func updateMonitors() {
        guard isEnabled else { return }
        report {
            try engine.setMonitors(NSScreen.screens)
            try engine.sync()
        }
        publish()
    }

    private func report(_ body: () throws -> Void) {
        do {
            try body()
        } catch {
            // A failing call repeats on every frame of a slide or pan; log
            // each distinct error at most every few seconds.
            let message = String(describing: error)
            let now = CACurrentMediaTime()
            if now - (lastLogged[message] ?? -.infinity) >= 5 {
                lastLogged[message] = now
                log.error("\(message, privacy: .public)")
            }
        }
        for line in Engine.takeLog() {
            log.notice("\(line, privacy: .public)")
        }
    }

    private static func describe(_ notification: Notification) -> String {
        let app = notification.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication
        let name = notification.name.rawValue
        guard let app else { return name }
        return "\(name) \(app.localizedName ?? "?") (\(app.processIdentifier))"
    }

    @objc private func logOnly(_ notification: Notification) {
        log.notice("\(Self.describe(notification), privacy: .public)")
    }

    @objc private func appLaunched(_ notification: Notification) {
        guard let app = notification.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication,
              app.activationPolicy == .regular else { return }
        let pid = app.processIdentifier
        engine.observeApp(pid)
        scheduleSync()
        // Freshly launched apps often refuse Accessibility requests for a
        // moment; observing again is a no-op once it has worked.
        for delay in [0.5, 2.0] {
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
                MainActor.assumeIsolated {
                    self?.engine.observeApp(pid)
                    self?.scheduleSync()
                }
            }
        }
    }

    @objc private func appTerminated(_ notification: Notification) {
        guard let app = notification.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication else { return }
        log.notice("\(Self.describe(notification), privacy: .public)")
        engine.forgetApp(app.processIdentifier)
        scheduleSync()
    }

    @objc private func appActivated(_ notification: Notification) {
        // Apps busy when observed at launch or Re-tile get another chance
        // each time they come forward; observing again is a no-op once it has worked.
        if let app = notification.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication,
           app.activationPolicy == .regular {
            engine.observeApp(app.processIdentifier)
        }
        scheduleSync()
    }

    @objc private func somethingChanged(_ notification: Notification) {
        log.notice("\(Self.describe(notification), privacy: .public)")
        if notification.name == NSWorkspace.activeSpaceDidChangeNotification {
            engine.noteTransition()
        }
        scheduleSync()
    }

    @objc private func screensChanged(_ notification: Notification) {
        let ids = NSScreen.screens.map { String($0.displayID) }.joined(separator: ", ")
        log.notice("screens changed: \(ids, privacy: .public)")
        engine.noteTransition()
        updateMonitors()
    }

    @objc private func slept(_ notification: Notification) {
        log.notice("sleep: \(notification.name.rawValue, privacy: .public)")
        engine.noteTransition()
    }

    /// Reads every monitor and window again as displays and apps come back
    /// over a few seconds; macOS moves windows itself when displays return.
    @objc private func woke(_ notification: Notification) {
        log.notice("wake: \(notification.name.rawValue, privacy: .public)")
        engine.noteTransition()
        for delay in [1.0, 5.0] {
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
                MainActor.assumeIsolated { self?.updateMonitors() }
            }
        }
    }
}

/// Calls `onFrame` once per display refresh while running, so slides are as
/// smooth as the display allows (120 Hz on ProMotion screens).
@MainActor
private final class FrameDriver: NSObject {
    private let onFrame: () -> Void
    private var link: CADisplayLink?
    private var timer: Timer?

    var isRunning: Bool { link != nil || timer != nil }

    init(onFrame: @escaping () -> Void) {
        self.onFrame = onFrame
    }

    func start() {
        guard !isRunning else { return }
        if let screen = NSScreen.main ?? NSScreen.screens.first {
            let link = screen.displayLink(target: self, selector: #selector(frame(_:)))
            link.add(to: .main, forMode: .common)
            self.link = link
        } else {
            let timer = Timer(timeInterval: 1.0 / 60, repeats: true) { [weak self] _ in
                MainActor.assumeIsolated { self?.onFrame() }
            }
            RunLoop.main.add(timer, forMode: .common)
            self.timer = timer
        }
    }

    func stop() {
        link?.invalidate()
        link = nil
        timer?.invalidate()
        timer = nil
    }

    @objc private func frame(_ link: CADisplayLink) {
        onFrame()
    }
}
