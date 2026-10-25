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
    private lazy var modifiers = ModifierWatcher { [weak self] in self?.minimap.hide() }
    private var syncScheduled = false

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
        report { try engine.setConfig(json: settings.engineConfigJSON) }
        updateMonitors()
        hotKeys.register(settings.hotKeys)
        modifiers.start()
    }

    /// Pauses global shortcuts, e.g. while one is being recorded.
    func setHotKeysPaused(_ paused: Bool) {
        if paused {
            hotKeys.unregisterAll()
        } else {
            hotKeys.register(settings.hotKeys)
        }
    }

    func perform(_ action: Action) {
        switch action {
        case .engine(let command, let arg):
            report { try engine.run(command, arg: arg) }
            if let state = publish() { minimap.show(state) }
        case .switcher:
            break
        }
    }

    private func applySettings() {
        report { try engine.setConfig(json: settings.engineConfigJSON) }
        hotKeys.register(settings.hotKeys)
        publish()
    }

    @discardableResult
    private func publish() -> EngineState? {
        guard let state = engine.state() else { return nil }
        onStateChange?(state)
        return state
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
