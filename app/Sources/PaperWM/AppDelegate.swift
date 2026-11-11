import AppKit
import SwiftUI
import CPaperWMEngine

/// Menu bar app hosting the window manager.
@main
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate, NSPopoverDelegate {
    private var manager: WindowManager?
    private var statusItem: NSStatusItem?
    /// Right-click menu, and the left-click one until tiling has started.
    private let menu = NSMenu()
    private let popover = NSPopover()
    private let panel = PanelModel()
    /// Closes the panel on a click in another app, which a transient popover
    /// of an app that is not active does not always notice.
    private var outsideClicks: Any?
    private var trustTimer: Timer?
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
        item.button?.target = self
        item.button?.action = #selector(statusItemClicked(_:))
        item.button?.sendAction(on: [.leftMouseUp, .rightMouseUp])
        menu.delegate = self
        statusItem = item
        popover.behavior = .transient
        popover.delegate = self
        popover.contentViewController = NSHostingController(rootView: MenuPanelView(model: panel))
        panel.perform = { [weak self] action in self?.panelAction(action) }

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
        manager.start()
    }

    @objc private func statusItemClicked(_ sender: NSStatusBarButton) {
        guard let manager, NSApp.currentEvent?.type != .rightMouseUp else {
            // Shows the menu as if the item owned it.
            statusItem?.menu = menu
            sender.performClick(nil)
            statusItem?.menu = nil
            return
        }
        if popover.isShown {
            popover.performClose(nil)
            return
        }
        refreshPanel(manager.engine.state())
        manager.onPublish = { [weak self] state in self?.refreshPanel(state) }
        popover.show(relativeTo: sender.bounds, of: sender, preferredEdge: .minY)
        outsideClicks = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) {
            [weak self] _ in
            MainActor.assumeIsolated { self?.popover.performClose(nil) }
        }
    }

    private func refreshPanel(_ state: EngineState?) {
        guard let manager else { return }
        panel.state = state
        panel.enabled = manager.isEnabled
        // The widths Cycle Column Width steps through, then full width.
        panel.presets = Set(manager.settings.values.widthPresets.filter { $0 > 0 && $0 < 1 } + [1]).sorted()
    }

    func popoverDidClose(_ notification: Notification) {
        if let outsideClicks { NSEvent.removeMonitor(outsideClicks) }
        outsideClicks = nil
        manager?.onPublish = nil
        panel.hovered = nil
    }

    private func panelAction(_ action: PanelAction) {
        guard let manager else { return }
        switch action {
        case .setWidth(let fraction):
            manager.perform(.engine(PWM_COMMAND_SET_WIDTH, fraction))
        case .arrange(let layout):
            manager.arrange(layout.columns.map { (rows: $0.rows, fraction: $0.fraction) })
        case .focus(let id):
            manager.perform(.engine(PWM_COMMAND_FOCUS_WINDOW, Double(id)))
        case .setEnabled(let enabled):
            manager.setEnabled(enabled)
            refreshPanel(enabled ? manager.engine.state() : nil)
        case .settings:
            popover.performClose(nil)
            openSettings()
        case .retile:
            retile()
        case .guide:
            popover.performClose(nil)
            showOnboarding()
        case .quit:
            NSApp.terminate(nil)
        }
    }

    // Rebuilt on every open so the permission state is current.
    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        menu.addItem(disabled("PaperWM \(Engine.version)"))

        if let manager {
            menu.addItem(.separator())
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
