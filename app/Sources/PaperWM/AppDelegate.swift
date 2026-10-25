import AppKit

/// Menu bar app hosting the engine. For now it offers the proof-of-concept
/// actions; keyboard control arrives with the scrollable layout.
@main
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
    private let engine = Engine()
    private var statusItem: NSStatusItem?

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

        if !Engine.isTrusted() {
            _ = Engine.isTrusted(prompt: true)
        }
    }

    // Rebuilt on every open so the permission state is current.
    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        menu.addItem(disabled("PaperWM \(Engine.version)"))

        if Engine.isTrusted() {
            menu.addItem(disabled("Accessibility access granted"))
            menu.addItem(.separator())
            menu.addItem(action("Tile Windows on This Screen", #selector(tileWindows)))
            menu.addItem(action("Show Windows…", #selector(showWindows)))
        } else {
            menu.addItem(disabled("Accessibility access needed"))
            menu.addItem(action("Grant Accessibility Access…", #selector(grantAccess)))
        }

        menu.addItem(.separator())
        menu.addItem(NSMenuItem(title: "Quit PaperWM", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q"))
    }

    @objc private func tileWindows() {
        guard let screen = NSScreen.main else { return }
        do {
            _ = try engine.tileColumns(in: screen.accessibilityVisibleFrame, gap: 8)
        } catch {
            show(error)
        }
    }

    @objc private func showWindows() {
        do {
            let lines = try engine.windows().map { w in
                let flags = [w.isStandard ? nil : "non-standard", w.isMinimized ? "minimized" : nil].compactMap { $0 }
                let suffix = flags.isEmpty ? "" : " (\(flags.joined(separator: ", ")))"
                return "\(w.appName): \(w.title.isEmpty ? "untitled" : w.title)\(suffix)"
            }
            alert(title: "\(lines.count) windows", text: lines.joined(separator: "\n"))
        } catch {
            show(error)
        }
    }

    @objc private func grantAccess() {
        _ = Engine.isTrusted(prompt: true)
        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility") {
            NSWorkspace.shared.open(url)
        }
    }

    private func show(_ error: Error) {
        alert(title: "PaperWM", text: String(describing: error))
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
