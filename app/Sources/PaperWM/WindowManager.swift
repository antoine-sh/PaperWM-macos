import AppKit
import CPaperWMEngine

/// Connects the engine to the system: tells it about monitors and apps,
/// syncs it when windows change, and routes hot keys to it.
@MainActor
final class WindowManager {
    let engine = Engine()
    let settings = Settings()
    private(set) lazy var hotKeys = HotKeyCenter { [weak self] action in
        self?.perform(action)
    }
    /// Called with the new layout after every sync or command.
    var onStateChange: ((EngineState) -> Void)?
    private let minimap = Minimap()
    private let switcher = Switcher()
    private let decorations = Decorations()
    private lazy var gestures = Gestures { [weak self] action in self?.perform(action) }
    private lazy var modifiers = ModifierWatcher { [weak self] in self?.superReleased() }
    private var syncScheduled = false
    private var hotKeysPaused = false
    /// What was last applied, so settings edits only redo what changed.
    private var appliedConfig = ""
    private var appliedShortcuts: [KeyCombo] = []
    /// Drives strip slides while one is running.
    private var animationTimer: Timer?
    private var lastTick: CFTimeInterval = 0

    func start() {
        let context = Unmanaged.passUnretained(self).toOpaque()
        engine.setChangeHandler({ context in
            guard let context else { return }
            let manager = Unmanaged<WindowManager>.fromOpaque(context).takeUnretainedValue()
            MainActor.assumeIsolated { manager.scheduleSync() }
        }, context: context)

        let workspace = NSWorkspace.shared.notificationCenter
        workspace.addObserver(self, selector: #selector(appLaunched(_:)),
                              name: NSWorkspace.didLaunchApplicationNotification, object: nil)
        workspace.addObserver(self, selector: #selector(appTerminated(_:)),
                              name: NSWorkspace.didTerminateApplicationNotification, object: nil)
        for name in [NSWorkspace.didActivateApplicationNotification, NSWorkspace.activeSpaceDidChangeNotification,
                     NSWorkspace.didUnhideApplicationNotification] {
            workspace.addObserver(self, selector: #selector(somethingChanged(_:)), name: name, object: nil)
        }
        NotificationCenter.default.addObserver(self, selector: #selector(screensChanged(_:)),
                                               name: NSApplication.didChangeScreenParametersNotification, object: nil)

        for app in NSWorkspace.shared.runningApplications where app.activationPolicy == .regular {
            engine.observeApp(app.processIdentifier)
        }
        settings.onChange = { [weak self] in self?.applySettings() }
        applySettings()
        updateMonitors()
        modifiers.start()
        gestures.start()
    }

    /// Pauses global shortcuts, e.g. while one is being recorded.
    func setHotKeysPaused(_ paused: Bool) {
        hotKeysPaused = paused
        if paused {
            hotKeys.unregisterAll()
        } else {
            hotKeys.register(settings.hotKeys)
        }
    }

    func perform(_ action: Action) {
        switch action {
        case .engine(PWM_COMMAND_TOGGLE_SCRATCH, _) where switcher.isOpen:
            // ⌃⌥Esc while switching cancels, as Esc does in PaperWM.
            switcher.cancel()
        case .engine(let command, let arg):
            report { try engine.run(command, arg: arg) }
            // Redrawing the minimap for every scroll event is too costly; it
            // shows when the pan ends.
            if let state = publish(), command != PWM_COMMAND_SCROLL_BY {
                minimap.show(state, style: settings.values.minimapStyle)
            }
        case .switcher(let forward):
            guard let state = engine.state() else { return }
            minimap.hide()
            switcher.step(forward: forward, state: state)
            // A quick tap that is already over switches straight away.
            if !ModifierWatcher.superHeld { superReleased() }
        case .toggle(let setting):
            settings.values[keyPath: setting].toggle()
        }
    }

    private func superReleased() {
        minimap.hide()
        if let id = switcher.commit() {
            perform(.engine(PWM_COMMAND_FOCUS_WINDOW, Double(id)))
            minimap.hide()
        }
    }

    private func applySettings() {
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
        }
        decorations.apply(settings.values)
    }

    @discardableResult
    private func publish() -> EngineState? {
        animate()
        decorations.refresh()
        guard let state = engine.state() else { return nil }
        onStateChange?(state)
        return state
    }

    /// Ticks the engine every frame until the strips stop sliding.
    private func animate() {
        guard animationTimer == nil, engine.tick(0) else { return }
        lastTick = CACurrentMediaTime()
        let timer = Timer(timeInterval: 1.0 / 60, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.step() }
        }
        RunLoop.main.add(timer, forMode: .common)
        animationTimer = timer
    }

    private func step() {
        // Real elapsed time, so slow Accessibility calls skip frames instead
        // of stretching the slide.
        let now = CACurrentMediaTime()
        let sliding = engine.tick(now - lastTick)
        lastTick = now
        if !sliding {
            animationTimer?.invalidate()
            animationTimer = nil
        }
    }

    /// Syncs on the next run loop turn, coalescing bursts of notifications.
    func scheduleSync() {
        guard !syncScheduled else { return }
        syncScheduled = true
        Task { @MainActor in
            self.syncScheduled = false
            self.syncNow()
        }
    }

    func syncNow() {
        report { try engine.sync() }
        publish()
    }

    private func updateMonitors() {
        report {
            try engine.setMonitors(NSScreen.screens.map(\.accessibilityVisibleFrame))
            try engine.sync()
        }
        publish()
    }

    private func report(_ body: () throws -> Void) {
        do {
            try body()
        } catch {
            NSLog("PaperWM: \(error)")
        }
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
        engine.forgetApp(app.processIdentifier)
        scheduleSync()
    }

    @objc private func somethingChanged(_ notification: Notification) {
        scheduleSync()
    }

    @objc private func screensChanged(_ notification: Notification) {
        updateMonitors()
    }
}
