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

    @State private var tab = Tab.general
    @State private var presetsText = ""
    @FocusState private var presetsFocused: Bool
    /// Read from macOS rather than stored, so it follows changes made in System Settings › Login Items.
    @State private var launchAtLogin = Self.isLaunchAtLogin

    private enum Tab: String, CaseIterable {
        case general = "General", layout = "Layout", appearance = "Appearance", apps = "Apps", shortcuts = "Shortcuts"

        var symbol: String {
            switch self {
            case .general: "gearshape"
            case .layout: "rectangle.split.3x1"
            case .appearance: "paintbrush"
            case .apps: "app.badge"
            case .shortcuts: "keyboard"
            }
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            tabBar
            Divider()
            Group {
                switch tab {
                case .general: general
                case .layout: layout
                case .appearance: appearance
                case .apps: apps
                case .shortcuts: shortcuts
                }
            }
            .formStyle(.grouped)
            .scrollContentBackground(.hidden)
            .toggleStyle(.switch)
        }
        .frame(width: 620, height: 520)
        .onAppear {
            presetsText = Self.format(settings.values.widthPresets)
            launchAtLogin = Self.isLaunchAtLogin
        }
    }

    /// The tabs as the menu bar panel's tiles.
    private var tabBar: some View {
        HStack(spacing: 8) {
            ForEach(Array(Tab.allCases.enumerated()), id: \.element) { index, item in
                Button { select(item) } label: {
                    VStack(spacing: 4) {
                        Image(systemName: item.symbol)
                            .font(.system(size: 16))
                            .frame(height: 20)
                        Text(item.rawValue).font(.caption)
                    }
                    .foregroundStyle(tab == item ? Color.accentColor : Color.primary)
                    .padding(4)
                    .frame(width: 92, height: 52)
                    .tile(selected: tab == item)
                }
                .buttonStyle(.plain)
                // Plain buttons take no keyboard focus by default.
                .keyboardShortcut(KeyEquivalent(Character("\(index + 1)")), modifiers: .command)
                .accessibilityLabel(item.rawValue)
                .accessibilityAddTraits(tab == item ? .isSelected : [])
            }
        }
        .frame(maxWidth: .infinity)
        .padding(16)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Settings sections")
    }

    /// Leaving a tab removes its views before focus moves, so commit the
    /// presets field and end a recording that would go on unseen.
    private func select(_ item: Tab) {
        if tab == .layout { commitPresets() }
        recorder.stop()
        tab = item
    }

    /// A section footer in the panel's footer style.
    private func note(_ text: String) -> some View {
        Text(text).font(.callout).foregroundStyle(.secondary)
    }

    /// Still on while it waits for approval in System Settings › Login Items.
    static var isLaunchAtLogin: Bool {
        [.enabled, .requiresApproval].contains(SMAppService.mainApp.status)
    }

    static func setLaunchAtLogin(_ on: Bool) {
        do {
            try on ? SMAppService.mainApp.register() : SMAppService.mainApp.unregister()
        } catch {
            NSLog("PaperWM: launch at login: \(error)")
        }
    }

    private var general: some View {
        Form {
            Section {
                Toggle("Launch at login", isOn: Binding(
                    get: { launchAtLogin },
                    set: { on in
                        Self.setLaunchAtLogin(on)
                        launchAtLogin = Self.isLaunchAtLogin
                    }
                ))
            } header: {
                PanelHeading(title: "Startup")
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
                PanelHeading(title: "Columns")
            } footer: {
                note("Width presets are percentages of the screen, separated by commas.")
            }
            Section {
                stepper("Animation", value: $settings.values.animationDuration, in: 0...1, step: 0.05) {
                    $0 < 0.001 ? "Off" : "\(Int(($0 * 1000).rounded())) ms"
                }
                Picker("Center focused column", selection: $settings.values.centerFocused) {
                    Text("Never").tag(CenterFocus.never)
                    Text("When columns overflow").tag(CenterFocus.onOverflow)
                    Text("Always").tag(CenterFocus.always)
                }
            } header: {
                PanelHeading(title: "Scrolling")
            }
        }
    }

    private func commitPresets() {
        if let presets = Self.parse(presetsText) { settings.values.widthPresets = presets }
        presetsText = Self.format(settings.values.widthPresets)
    }

    private var appearance: some View {
        Form {
            Section {
                Toggle("Draw a border around the focused window", isOn: $settings.values.focusBorder)
                Group {
                    ColorPicker("Color", selection: borderColor, supportsOpacity: false)
                    stepper("Width", value: $settings.values.borderWidth, in: 1...16, step: 1) { "\(Int($0)) pt" }
                }
                .disabled(!settings.values.focusBorder)
            } header: {
                PanelHeading(title: "Focus border")
            }
            Section {
                Toggle("Dim windows behind the focused one", isOn: $settings.values.dimBackground)
                stepper("Amount", value: $settings.values.dimAmount, in: 0.05...0.8, step: 0.05) { Self.percent($0) }
                    .disabled(!settings.values.dimBackground)
            } header: {
                PanelHeading(title: "Dimming")
            }
            Section {
                stepper("Window corner radius", value: $settings.values.cornerRadius, in: 0...40, step: 1) {
                    "\(Int($0)) pt"
                }
                .disabled(!settings.values.focusBorder && !settings.values.dimBackground)
            } header: {
                PanelHeading(title: "Corners")
            } footer: {
                note(Thumbnails.hasAccess
                     ? "Shapes the border and dimming until each window's own corners are measured."
                     : "Shapes the border and dimming. Grant Screen Recording permission to match "
                       + "each window's own corners.")
            }
            Section {
                Picker("Style", selection: $settings.values.minimapStyle) {
                    Text("Strip").tag(MinimapStyle.strip)
                    Text("App icon toolbar").tag(MinimapStyle.toolbar)
                }
            } header: {
                PanelHeading(title: "Minimap")
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
                Stepper(title, value: value, in: range, step: step)
                    .labelsHidden()
                    .accessibilityValue(shown)
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
                    AppRuleRow(rule: binding(for: rule)) {
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
                PanelHeading(title: "App rules")
            } footer: {
                note("Floating apps are left where they are, like dialogs. "
                     + "An app with a slot opens at that column of its display's strip; you can still move it after.")
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
                Section {
                    ForEach(HotKey.defaults.filter { $0.category == category }, id: \.id) { shortcutRow($0) }
                } header: {
                    PanelHeading(title: category.rawValue)
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
            let recording = recorder.recording == hotKey.id
            Button {
                if recording {
                    recorder.stop()
                } else {
                    recorder.start(hotKey.id) { combo.wrappedValue = $0 }
                }
            } label: {
                Text(recording ? "Press keys…" : combo.wrappedValue.symbol)
                    .font(.system(.body, design: .rounded))
                    .foregroundStyle(recording ? Color.accentColor : Color.primary)
                    .frame(minWidth: 96, minHeight: 22)
                    .padding(4)
                    .tile(selected: recording)
            }
            .buttonStyle(.plain)
            .help(recording ? "Press the new shortcut, or Esc to cancel" : "Click to record a new shortcut")
            .accessibilityLabel("\(hotKey.title) shortcut")
            .accessibilityValue(recording ? "Recording" : combo.wrappedValue.symbol)
            Button { settings.values.shortcuts[hotKey.id] = nil } label: {
                Image(systemName: "arrow.uturn.backward")
            }
            .buttonStyle(.borderless)
            .help("Reset to the default shortcut")
            .accessibilityLabel("Reset")
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

/// One app's rule: float it, or pin it to a column.
private struct AppRuleRow: View {
    @Binding var rule: AppRule
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

@MainActor
extension NSWindow {
    /// Brings the window of this menu bar app in front of the active app's.
    func present() {
        NSApp.activate()
        makeKeyAndOrderFront(nil)
        // macOS 14 may decline the activation above, leaving the window
        // behind the front app's windows.
        orderFrontRegardless()
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
            window.titlebarAppearsTransparent = true
            window.isReleasedWhenClosed = false
            window.delegate = self
            self.window = window
        }
        window?.center()
        window?.present()
    }

    // Closing mid-recording must give the global shortcuts back.
    func windowWillClose(_ notification: Notification) {
        recorder?.stop()
    }
}
