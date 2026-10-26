import AppKit
import Carbon.HIToolbox
import ServiceManagement
import SwiftUI
import UniformTypeIdentifiers

/// The Settings window: general, layout, appearance, app rules and shortcuts.
struct SettingsView: View {
    @ObservedObject var settings: Settings
    /// Shortcut ids macOS refused to register.
    let failed: Set<String>
    @ObservedObject var recorder: ShortcutRecorder

    @State private var presetsText = ""
    @FocusState private var presetsFocused: Bool
    /// Read from macOS rather than stored, so it follows changes made in System Settings › Login Items.
    @State private var launchAtLogin = Self.isLaunchAtLogin

    var body: some View {
        TabView {
            general.tabItem { Label("General", systemImage: "gearshape") }
            layout.tabItem { Label("Layout", systemImage: "rectangle.split.3x1") }
            appearance.tabItem { Label("Appearance", systemImage: "paintbrush") }
            apps.tabItem { Label("Apps", systemImage: "app.badge") }
            shortcuts.tabItem { Label("Shortcuts", systemImage: "keyboard") }
        }
        .formStyle(.grouped)
        .frame(width: 620, height: 520)
        .onAppear {
            presetsText = Self.format(settings.values.widthPresets)
            launchAtLogin = Self.isLaunchAtLogin
        }
    }

    /// Still on while it waits for approval in System Settings › Login Items.
    private static var isLaunchAtLogin: Bool {
        [.enabled, .requiresApproval].contains(SMAppService.mainApp.status)
    }

    private var general: some View {
        Form {
            Section {
                Toggle("Launch at login", isOn: Binding(
                    get: { launchAtLogin },
                    set: { on in
                        do {
                            try on ? SMAppService.mainApp.register() : SMAppService.mainApp.unregister()
                        } catch {
                            NSLog("PaperWM: launch at login: \(error)")
                        }
                        launchAtLogin = Self.isLaunchAtLogin
                    }
                ))
            }
            Section("Workspace names") {
                ForEach(0..<settings.values.workspaceNames.count, id: \.self) { index in
                    TextField("Workspace \(index + 1)", text: $settings.values.workspaceNames[index])
                }
            }
        }
    }

    private var layout: some View {
        Form {
            Section {
                stepper("Gap", value: $settings.values.gap, in: 0...64, step: 2) { "\(Int($0)) pt" }
                TextField("Width presets", text: $presetsText)
                    .focused($presetsFocused)
                    .onSubmit { commitPresets() }
                    .onChange(of: presetsFocused) { _, focused in if !focused { commitPresets() } }
                stepper("Widen / narrow step", value: $settings.values.widthStep, in: 0.05...0.5, step: 0.05) {
                    "\(Self.percent($0)) of screen"
                }
                stepper("Taller / shorter step", value: $settings.values.heightStep, in: 0.05...0.5, step: 0.05) {
                    "\(Self.percent($0)) of column"
                }
                Toggle("Keep original height of single windows", isOn: $settings.values.keepOriginalHeight)
            } header: {
                Text("Columns")
            } footer: {
                Text("Width presets are percentages of the screen, separated by commas.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Section("Scrolling") {
                stepper("Animation", value: $settings.values.animationDuration, in: 0...1, step: 0.05) {
                    $0 < 0.001 ? "Off" : "\(Int(($0 * 1000).rounded())) ms"
                }
                Picker("Center focused column", selection: $settings.values.centerFocused) {
                    Text("Never").tag(CenterFocus.never)
                    Text("When columns overflow").tag(CenterFocus.onOverflow)
                    Text("Always").tag(CenterFocus.always)
                }
            }
        }
    }

    private func commitPresets() {
        if let presets = Self.parse(presetsText) { settings.values.widthPresets = presets }
        presetsText = Self.format(settings.values.widthPresets)
    }

    private var appearance: some View {
        Form {
            Section("Focus border") {
                Toggle("Draw a border around the focused window", isOn: $settings.values.focusBorder)
                Group {
                    ColorPicker("Color", selection: borderColor, supportsOpacity: false)
                    stepper("Width", value: $settings.values.borderWidth, in: 1...16, step: 1) { "\(Int($0)) pt" }
                }
                .disabled(!settings.values.focusBorder)
            }
            Section("Dimming") {
                Toggle("Dim windows behind the focused one", isOn: $settings.values.dimBackground)
                stepper("Amount", value: $settings.values.dimAmount, in: 0.05...0.8, step: 0.05) { Self.percent($0) }
                    .disabled(!settings.values.dimBackground)
            }
            Section {
                stepper("Window corner radius", value: $settings.values.cornerRadius, in: 0...40, step: 1) {
                    "\(Int($0)) pt"
                }
                .disabled(!settings.values.focusBorder && !settings.values.dimBackground)
            } footer: {
                Text(Thumbnails.hasAccess
                     ? "Shapes the border and dimming until each window's own corners are measured."
                     : "Shapes the border and dimming. Grant Screen Recording permission to match "
                       + "each window's own corners.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Section("Minimap") {
                Picker("Style", selection: $settings.values.minimapStyle) {
                    Text("Strip").tag(MinimapStyle.strip)
                    Text("App icon toolbar").tag(MinimapStyle.toolbar)
                }
            }
        }
    }

    /// A numeric setting with its current value shown beside the stepper.
    private func stepper(_ title: String, value: Binding<Double>, in range: ClosedRange<Double>, step: Double,
                         format: (Double) -> String) -> some View {
        let shown = format(value.wrappedValue)
        return LabeledContent(title) {
            HStack {
                Text(shown).monospacedDigit()
                Stepper(title, value: value, in: range, step: step).labelsHidden()
            }
        }
    }

    private static func percent(_ fraction: Double) -> String {
        "\(Int((fraction * 100).rounded()))%"
    }

    private var borderColor: Binding<Color> {
        Binding(
            get: { Color(nsColor: NSColor(hex: settings.values.borderColor) ?? .controlAccentColor) },
            set: { settings.values.borderColor = NSColor($0).hex }
        )
    }

    private var apps: some View {
        Form {
            Section {
                if settings.values.appRules.isEmpty {
                    Text("Every app tiles.")
                        .foregroundStyle(.secondary)
                }
                ForEach(settings.values.appRules) { rule in
                    AppRuleRow(rule: binding(for: rule), workspaceNames: settings.values.workspaceNames) {
                        settings.values.appRules.removeAll { $0.id == rule.id }
                    }
                }
                Menu("Add App") {
                    ForEach(runningApps, id: \.name) { app in
                        Button {
                            addRule(app: app.name, bundleID: app.bundleID)
                        } label: {
                            Label {
                                Text(app.name)
                            } icon: {
                                Image(nsImage: AppIcons.small(bundleID: app.bundleID, name: app.name))
                            }
                        }
                    }
                    if !runningApps.isEmpty { Divider() }
                    Button("Other App…") { chooseApp() }
                }
                .fixedSize()
            } header: {
                Text("App rules")
            } footer: {
                Text("Floating apps are left where they are, on every workspace, like dialogs. "
                     + "An app with a slot opens at that column of that workspace; you can still move it after.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private struct RunningApp {
        let name: String
        let bundleID: String?
    }

    /// Running apps without a rule yet, by name.
    private var runningApps: [RunningApp] {
        let taken = Set(settings.values.appRules.map(\.app))
        var seen = Set<String>()
        return NSWorkspace.shared.runningApplications
            .filter { $0.activationPolicy == .regular && $0.processIdentifier != getpid() }
            .compactMap { app in
                app.localizedName.map { RunningApp(name: $0, bundleID: app.bundleIdentifier) }
            }
            .filter { !taken.contains($0.name) && seen.insert($0.name).inserted }
            .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }

    /// Edits the rule with `rule`'s id, which survives others being removed.
    private func binding(for rule: AppRule) -> Binding<AppRule> {
        Binding(
            get: { settings.values.appRules.first { $0.id == rule.id } ?? rule },
            set: { edited in
                guard let index = settings.values.appRules.firstIndex(where: { $0.id == rule.id }) else { return }
                settings.values.appRules[index] = edited
            }
        )
    }

    private func addRule(app: String, bundleID: String?) {
        guard !settings.values.appRules.contains(where: { $0.app == app }) else { return }
        settings.values.appRules.append(AppRule(app: app, bundleID: bundleID))
    }

    /// Picks an app that is not running from the Applications folder.
    private func chooseApp() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.application]
        panel.directoryURL = URL(fileURLWithPath: "/Applications")
        panel.prompt = "Add"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        // Windows report the app's bundle name once it runs, which can
        // differ from its file name ("Code" for Visual Studio Code.app).
        let bundle = Bundle(url: url)
        let info = (bundle?.localizedInfoDictionary ?? [:]).merging(bundle?.infoDictionary ?? [:]) { local, _ in local }
        var name = (info["CFBundleDisplayName"] ?? info["CFBundleName"]) as? String
            ?? FileManager.default.displayName(atPath: url.path)
        if name.hasSuffix(".app") { name.removeLast(4) }
        addRule(app: name, bundleID: bundle?.bundleIdentifier)
    }

    private var shortcuts: some View {
        Form {
            ForEach(HotKey.Category.allCases, id: \.self) { category in
                Section(category.rawValue) {
                    ForEach(HotKey.defaults.filter { $0.category == category }, id: \.id) { shortcutRow($0) }
                }
            }
        }
    }

    private func shortcutRow(_ hotKey: HotKey) -> some View {
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

/// One app's rule: float it, or pin it to a workspace and column.
private struct AppRuleRow: View {
    @Binding var rule: AppRule
    let workspaceNames: [String]
    let onRemove: () -> Void

    var body: some View {
        HStack {
            Image(nsImage: AppIcons.small(bundleID: rule.bundleID, name: rule.app))
            Text(rule.app).lineLimit(1)
            Spacer()
            Picker("Placement", selection: $rule.action) {
                Text("Float").tag(AppRule.Action.float)
                Text("Fixed slot").tag(AppRule.Action.slot)
            }
            .labelsHidden()
            .fixedSize()
            if rule.action == .slot {
                Picker("Workspace", selection: $rule.workspace) {
                    ForEach(0..<workspaceNames.count, id: \.self) { index in
                        Text(workspaceNames[index].isEmpty ? "Workspace \(index + 1)" : workspaceNames[index])
                            .tag(index)
                    }
                }
                .labelsHidden()
                .fixedSize()
                Stepper("Column \(rule.column + 1)", value: $rule.column, in: 0...19)
                    .fixedSize()
            }
            Button(action: onRemove) { Image(systemName: "minus.circle") }
                .buttonStyle(.borderless)
                .help("Remove the rule")
        }
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
