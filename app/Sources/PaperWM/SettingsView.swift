import AppKit
import Carbon.HIToolbox
import SwiftUI

/// The Settings window: layout, workspace names and shortcuts.
struct SettingsView: View {
    @ObservedObject var settings: Settings
    /// Shortcut ids macOS refused to register.
    let failed: Set<String>
    @ObservedObject var recorder: ShortcutRecorder

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
                let combo = binding(for: hotKey)
                Button(recorder.recording == hotKey.id ? "Press keys…" : combo.wrappedValue.symbol) {
                    if recorder.recording == hotKey.id {
                        recorder.stop()
                    } else {
                        recorder.start(hotKey.id) { combo.wrappedValue = $0 }
                    }
                }
                .frame(minWidth: 110)
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

/// Records one shortcut at a time: after `start`, the next key press with
/// ⌃, ⌥ or ⌘ becomes the shortcut, and Esc cancels. Global shortcuts step
/// aside while recording, since they would swallow the keys.
@MainActor
final class ShortcutRecorder: ObservableObject {
    /// Id of the shortcut being recorded.
    @Published private(set) var recording: String?
    private var monitor: Any?
    private let onRecording: (Bool) -> Void

    init(onRecording: @escaping (Bool) -> Void) {
        self.onRecording = onRecording
    }

    func start(_ id: String, onCombo: @escaping (KeyCombo) -> Void) {
        stop()
        recording = id
        onRecording(true)
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            MainActor.assumeIsolated {
                let modifiers = Self.carbonModifiers(event.modifierFlags)
                if event.keyCode == UInt16(kVK_Escape) && modifiers == 0 {
                    self?.stop()
                } else if modifiers & ~shiftKey != 0 {
                    onCombo(KeyCombo(keyCode: Int(event.keyCode), modifiers: modifiers))
                    self?.stop()
                }
            }
            return nil
        }
    }

    func stop() {
        guard let monitor else { return }
        NSEvent.removeMonitor(monitor)
        self.monitor = nil
        recording = nil
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
final class SettingsWindowController: NSObject, NSWindowDelegate {
    private var window: NSWindow?
    private var recorder: ShortcutRecorder?

    func show(settings: Settings, failed: Set<String>, onRecording: @escaping (Bool) -> Void) {
        recorder?.stop()
        let recorder = ShortcutRecorder(onRecording: onRecording)
        self.recorder = recorder
        let view = SettingsView(settings: settings, failed: failed, recorder: recorder)
        if let window {
            window.contentViewController = NSHostingController(rootView: view)
        } else {
            let window = NSWindow(contentViewController: NSHostingController(rootView: view))
            window.title = "PaperWM Settings"
            window.styleMask = [.titled, .closable]
            window.isReleasedWhenClosed = false
            window.delegate = self
            self.window = window
        }
        NSApp.activate()
        window?.center()
        window?.makeKeyAndOrderFront(nil)
    }

    // Closing mid-recording must give the global shortcuts back.
    func windowWillClose(_ notification: Notification) {
        recorder?.stop()
    }
}
