import AppKit
import CPaperWMEngine

/// Menu bar app hosting the window manager.
@main
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
    private var manager: WindowManager?
    private var statusItem: NSStatusItem?
    private var trustTimer: Timer?
    private var state: EngineState?
    private let settingsWindow = SettingsWindowController()
    private let onboarding = OnboardingWindowController()

    static func main() {
        let app = NSApplication.shared
        let delegate = AppDelegate()
        app.delegate = delegate
        app.setActivationPolicy(.accessory)
        withExtendedLifetime(delegate) { app.run() }
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        item.button?.image = NSImage(systemSymbolName: "rectangle.split.3x1", accessibilityDescription: "PaperWM")
        let menu = NSMenu()
        menu.delegate = self
        item.menu = menu
        statusItem = item

        if Engine.isTrusted() {
            startManager()
            onboarding.resumeIfInterrupted()
        } else {
            // A first launch, or a rebuild that lost the permission: the guide explains and asks for it.
            onboarding.show()
            waitForTrust()
        }
    }

    /// Starts tiling as soon as the user grants Accessibility access.
    private func waitForTrust() {
        trustTimer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] timer in
            MainActor.assumeIsolated {
                guard Engine.isTrusted() else { return }
                timer.invalidate()
                self?.startManager()
            }
        }
    }

    private func startManager() {
        guard manager == nil else { return }
        let manager = WindowManager()
        self.manager = manager
        manager.onStateChange = { [weak self] state in self?.show(state) }
        manager.start()
    }

    // Rebuilt on every open so the permission state is current.
    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        menu.addItem(disabled("PaperWM \(Engine.version)"))

        if let manager {
            menu.addItem(.separator())
            if let state {
                for (index, workspace) in state.workspaces.enumerated() {
                    let item = action(workspace.name, #selector(switchWorkspace(_:)))
                    item.tag = index
                    item.state = index == state.activeWorkspace ? .on : .off
                    menu.addItem(item)
                }
                menu.addItem(.separator())
            }
            let border = action("Focus Border", #selector(toggleSetting(_:)))
            border.representedObject = \Settings.Values.focusBorder
            border.state = manager.settings.values.focusBorder ? .on : .off
            menu.addItem(border)
            let dim = action("Dim Background Windows", #selector(toggleSetting(_:)))
            dim.representedObject = \Settings.Values.dimBackground
            dim.state = manager.settings.values.dimBackground ? .on : .off
            menu.addItem(dim)
            menu.addItem(.separator())
            menu.addItem(action("Re-tile Now", #selector(retile)))
            let shortcuts = NSMenuItem(title: "Keyboard Shortcuts", action: nil, keyEquivalent: "")
            let submenu = NSMenu()
            for hotKey in manager.hotKeys.hotKeys {
                submenu.addItem(disabled("\(hotKey.combo.symbol)\t\(hotKey.title)"))
            }
            shortcuts.submenu = submenu
            menu.addItem(shortcuts)
            menu.addItem(action("Show Windows…", #selector(showWindows)))
            menu.addItem(action("Welcome Guide…", #selector(showOnboarding)))
            let settings = action("Settings…", #selector(openSettings))
            settings.keyEquivalent = ","
            menu.addItem(settings)
        } else {
            menu.addItem(disabled("Accessibility access needed"))
            menu.addItem(action("Grant Accessibility Access…", #selector(grantAccess)))
        }

        menu.addItem(.separator())
        menu.addItem(NSMenuItem(title: "Quit PaperWM", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q"))
    }

    /// Shows the active workspace's name next to the menu bar icon.
    private func show(_ state: EngineState) {
        self.state = state
        let name = state.activeWorkspace.map { state.workspaces[$0].name } ?? ""
        statusItem?.button?.title = name.isEmpty ? "" : " \(name)"
        statusItem?.button?.imagePosition = .imageLeading
    }

    @objc private func switchWorkspace(_ sender: NSMenuItem) {
        manager?.perform(.engine(PWM_COMMAND_SWITCH_WORKSPACE, Double(sender.tag)))
    }

    @objc private func toggleSetting(_ sender: NSMenuItem) {
        guard let setting = sender.representedObject as? WritableKeyPath<Settings.Values, Bool> else { return }
        manager?.perform(.toggle(setting))
    }

    @objc private func openSettings() {
        guard let manager else { return }
        settingsWindow.show(settings: manager.settings, failed: manager.hotKeys.failed) { [weak manager] recording in
            manager?.setHotKeysPaused(recording)
        }
    }

    @objc private func retile() {
        manager?.retile()
    }

    @objc private func showWindows() {
        guard let manager else { return }
        do {
            let lines = try manager.engine.windows().map { w in
                let flags = [w.isStandard && w.isResizable ? nil : "floating", w.isMinimized ? "minimized" : nil]
                    .compactMap { $0 }
                let suffix = flags.isEmpty ? "" : " (\(flags.joined(separator: ", ")))"
                return "\(w.appName): \(w.title.isEmpty ? "untitled" : w.title)\(suffix)"
            }
            alert(title: "\(lines.count) windows", text: lines.joined(separator: "\n"))
        } catch {
            alert(title: "PaperWM", text: String(describing: error))
        }
    }

    @objc private func showOnboarding() {
        onboarding.show()
    }

    @objc private func grantAccess() {
        onboarding.show(at: .accessibility)
    }

    private func alert(title: String, text: String) {
        NSApp.activate()
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = text
        alert.runModal()
    }

    private func disabled(_ title: String) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        item.isEnabled = false
        return item
    }

    private func action(_ title: String, _ selector: Selector) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: selector, keyEquivalent: "")
        item.target = self
        return item
    }
}
