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
    private lazy var tabBars = TabBars { [weak self] id in
        self?.perform(.engine(PWM_COMMAND_FOCUS_WINDOW, Double(id)))
    }
    private lazy var gestures = Gestures { [weak self] action in self?.perform(action) }
    private lazy var modifiers = ModifierWatcher { [weak self] in self?.superReleased() }
    private lazy var drags = DragWatcher { [weak self] point in self?.dropped(at: point) }
    private var syncScheduled = false
    /// A sync asked for during a slide or a drag, run when it ends.
    private var syncDeferred = false
    private var hotKeysPaused = false
    /// What was last applied, so settings edits only redo what changed.
    private var appliedConfig = ""
    private var appliedShortcuts: [KeyCombo] = []
    /// Drives strip slides while one is running, once per display frame.
    private lazy var frames = FrameDriver { [weak self] in self?.step() }
    private var lastTick: CFTimeInterval = 0
    /// When each distinct error was last logged, to keep repeats out of the log.
    private var lastLogged: [String: CFTimeInterval] = [:]

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
        drags.start()
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
        case .engine(PWM_COMMAND_SCROLL_BY, let arg):
            // Pans arrive every frame: move the windows and the decorations,
            // and leave the full update to the end of the pan.
            report { try engine.run(PWM_COMMAND_SCROLL_BY, arg: arg) }
            decorations.track()
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
            if !ModifierWatcher.superHeld { superReleased() }
        case .toggle(let setting):
            settings.values[keyPath: setting].toggle()
        }
    }

    private func superReleased() {
        minimap.hide()
        if let id = switcher.commit() {
            perform(.engine(PWM_COMMAND_FOCUS_WINDOW, Double(id)))
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
        guard let state = engine.state() else {
            decorations.refresh(focus: nil)
            return nil
        }
        decorations.refresh(focus: state.focusedWindow)
        tabBars.update(state)
        onStateChange?(state)
        return state
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
        decorations.track()
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
        guard !syncScheduled else { return }
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
        report { try engine.sync() }
        publish()
    }

    /// Moves a window dragged onto another column there; the drop syncs in
    /// place of any sync held back during the drag.
    private func dropped(at point: CGPoint) {
        syncDeferred = false
        report { try engine.drop(at: point) }
        publish()
    }

    /// Puts every window back in its place, including ones that were moved.
    func retile() {
        report { try engine.retile() }
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
            // A failing call repeats on every frame of a slide or pan; log
            // each distinct error at most every few seconds.
            let message = String(describing: error)
            let now = CACurrentMediaTime()
            if now - (lastLogged[message] ?? -.infinity) >= 5 {
                lastLogged[message] = now
                NSLog("PaperWM: \(message)")
            }
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
