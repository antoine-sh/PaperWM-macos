import AppKit
import Carbon.HIToolbox
import SwiftUI

/// The Settings window: layout, workspace names and shortcuts.
struct SettingsView: View {
    @ObservedObject var settings: Settings
    /// Shortcut ids macOS refused to register.
    let failed: Set<String>
    /// Called with true while a shortcut is being recorded, so global
    /// shortcuts can step aside.
    let onRecording: (Bool) -> Void

    @State private var presetsText = ""

    var body: some View {
        TabView {
            layout.tabItem { Text("Layout") }
            shortcuts.tabItem { Text("Shortcuts") }
        }
        .padding()
        .frame(width: 560, height: 520)
        .onAppear { presetsText = Self.format(settings.values.widthPresets) }
    }

    private var layout: some View {
        Form {
            Stepper("Gap: \(Int(settings.values.gap)) pt", value: $settings.values.gap, in: 0...64, step: 2)
            TextField("Column widths (% of screen)", text: $presetsText)
                .onSubmit {
                    if let presets = Self.parse(presetsText) { settings.values.widthPresets = presets }
                    presetsText = Self.format(settings.values.widthPresets)
                }
            Section("Workspace names") {
                ForEach(0..<settings.values.workspaceNames.count, id: \.self) { index in
                    TextField("Workspace \(index + 1)", text: $settings.values.workspaceNames[index])
                }
            }
        }
    }

    private var shortcuts: some View {
        List(HotKey.defaults, id: \.id) { hotKey in
            HStack {
                Text(hotKey.title)
                Spacer()
                if failed.contains(hotKey.id) {
                    Image(systemName: "exclamationmark.triangle")
                        .foregroundStyle(.orange)
                        .help("Another app uses this shortcut")
                }
                ShortcutRecorder(combo: binding(for: hotKey), onRecording: onRecording)
                Button("Reset") { settings.values.shortcuts[hotKey.id] = nil }
                    .disabled(settings.values.shortcuts[hotKey.id] == nil)
            }
        }
    }

    private func binding(for hotKey: HotKey) -> Binding<KeyCombo> {
        Binding(
            get: { settings.values.shortcuts[hotKey.id] ?? hotKey.combo },
            set: { settings.values.shortcuts[hotKey.id] = $0 == hotKey.combo ? nil : $0 }
        )
    }

    static func format(_ presets: [Double]) -> String {
        presets.map { String(Int(($0 * 100).rounded())) }.joined(separator: ", ")
    }

    /// "33, 50, 67" → [0.33, 0.5, 0.67]; nil unless every entry is 1–100.
    static func parse(_ text: String) -> [Double]? {
        let parts = text.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }
        let values = parts.compactMap(Double.init)
        guard !values.isEmpty, values.count == parts.count, values.allSatisfy({ $0 >= 1 && $0 <= 100 }) else {
            return nil
        }
        return values.map { $0 / 100 }
    }
}

/// A button showing a shortcut; click it and press keys to change it, or
/// Esc to cancel.
struct ShortcutRecorder: View {
    @Binding var combo: KeyCombo
    let onRecording: (Bool) -> Void
    @State private var monitor: Any?

    var body: some View {
        Button(monitor == nil ? combo.symbol : "Press keys…") {
            monitor == nil ? start() : stop()
        }
        .frame(minWidth: 110)
    }

    private func start() {
        onRecording(true)
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            let modifiers = Self.carbonModifiers(event.modifierFlags)
            if event.keyCode == UInt16(kVK_Escape) && modifiers == 0 {
                stop()
            } else if modifiers & ~shiftKey != 0 {
                // A global shortcut needs ⌃, ⌥ or ⌘.
                combo = KeyCombo(keyCode: Int(event.keyCode), modifiers: modifiers)
                stop()
            }
            return nil
        }
    }

    private func stop() {
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
        onRecording(false)
    }

    static func carbonModifiers(_ flags: NSEvent.ModifierFlags) -> Int {
        var modifiers = 0
        if flags.contains(.control) { modifiers |= controlKey }
        if flags.contains(.option) { modifiers |= optionKey }
        if flags.contains(.shift) { modifiers |= shiftKey }
        if flags.contains(.command) { modifiers |= cmdKey }
        return modifiers
    }
}

/// Owns the Settings window so it can be reopened.
@MainActor
final class SettingsWindowController {
    private var window: NSWindow?

    func show(settings: Settings, failed: Set<String>, onRecording: @escaping (Bool) -> Void) {
        let view = SettingsView(settings: settings, failed: failed, onRecording: onRecording)
        if let window {
            window.contentViewController = NSHostingController(rootView: view)
        } else {
            let window = NSWindow(contentViewController: NSHostingController(rootView: view))
            window.title = "PaperWM Settings"
            window.styleMask = [.titled, .closable]
            window.isReleasedWhenClosed = false
            self.window = window
        }
        NSApp.activate()
        window?.center()
        window?.makeKeyAndOrderFront(nil)
    }
}
