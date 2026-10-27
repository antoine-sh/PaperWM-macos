import AppKit
import SwiftUI

/// What the welcome guide shows. Its controller refreshes the permissions
/// while it is open, since they change in System Settings.
@MainActor
final class OnboardingModel: ObservableObject {
    enum Step: Int, CaseIterable {
        case welcome, accessibility, screenRecording, shortcuts
    }

    @Published var step = Step.welcome
    @Published var trusted = false
    @Published var screenRecording = false
    /// Shortcut symbols by `HotKey.id`, with the user's overrides.
    @Published var combos: [String: String] = [:]
}

/// Walks a new user through what PaperWM does, the permissions it needs and
/// the first shortcuts to learn.
struct OnboardingView: View {
    @ObservedObject var model: OnboardingModel
    let onDone: () -> Void

    @State private var launchAtLogin = SettingsView.isLaunchAtLogin

    var body: some View {
        VStack(spacing: 0) {
            Group {
                switch model.step {
                case .welcome: welcome
                case .accessibility: accessibility
                case .screenRecording: screenRecording
                case .shortcuts: shortcuts
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .padding(32)

            Divider()
            HStack {
                Button("Back") { go(-1) }
                    .opacity(model.step == .welcome ? 0 : 1)
                    .disabled(model.step == .welcome)
                Spacer()
                HStack(spacing: 6) {
                    ForEach(OnboardingModel.Step.allCases, id: \.self) { step in
                        Circle()
                            .fill(step == model.step ? Color.accentColor : Color.secondary.opacity(0.3))
                            .frame(width: 6, height: 6)
                    }
                }
                Spacer()
                if model.step == .shortcuts {
                    Button("Done", action: onDone).keyboardShortcut(.defaultAction)
                } else {
                    Button(model.step == .screenRecording && !model.screenRecording ? "Skip" : "Continue") { go(1) }
                        .keyboardShortcut(.defaultAction)
                        // Nothing works without Accessibility, so there is nothing to show past it.
                        .disabled(model.step == .accessibility && !model.trusted)
                }
            }
            .padding()
        }
        .frame(width: 540, height: 480)
        .onAppear { launchAtLogin = SettingsView.isLaunchAtLogin }
    }

    private func go(_ offset: Int) {
        if let step = OnboardingModel.Step(rawValue: model.step.rawValue + offset) { model.step = step }
    }

    private var welcome: some View {
        VStack(spacing: 16) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .frame(width: 96, height: 96)
            Text("Welcome to PaperWM").font(.largeTitle.bold())
            Text("Your windows, laid out side by side on an endless strip of paper.")
                .foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 12) {
                feature("rectangle.split.3x1", "Every new window opens as a column to the right of the one you're on.")
                feature("arrow.left.and.right", "Move along the strip with the keyboard or the trackpad; it slides "
                    + "the window you want into view.")
                feature("square.grid.3x3", "Nine workspaces per monitor, independent of macOS Spaces: keep "
                    + "everything on one Space.")
            }
            .padding(.top, 8)
        }
    }

    private var accessibility: some View {
        permission(
            symbol: "accessibility",
            title: "Allow Accessibility access",
            text: "PaperWM uses it to move, resize and focus other apps' windows, and to read the trackpad for "
                + "panning. It is the only permission PaperWM needs, and System Integrity Protection stays on.",
            granted: model.trusted,
            hint: "Turn on PaperWM in Privacy & Security › Accessibility. If it is already on, remove it with − "
                + "and add it again: a rebuilt app counts as a new one. This page updates by itself.",
            grant: {
                _ = Engine.isTrusted(prompt: true)
                OnboardingView.openPrivacySettings("Privacy_Accessibility")
            }
        )
    }

    private var screenRecording: some View {
        permission(
            symbol: "rectangle.dashed.badge.record",
            title: "Screen Recording (optional)",
            text: "Lets the window switcher show live previews, and the focus border match each window's rounded "
                + "corners. Without it you get app icons and the corner radius set in Settings.",
            granted: model.screenRecording,
            hint: "Turn on PaperWM in Privacy & Security › Screen Recording. macOS may ask you to quit and "
                + "reopen PaperWM.",
            grant: {
                if !Thumbnails.askForAccessOnce() { OnboardingView.openPrivacySettings("Privacy_ScreenCapture") }
            }
        )
    }

    private var shortcuts: some View {
        VStack(spacing: 16) {
            Image(systemName: "keyboard").font(.system(size: 44)).foregroundStyle(Color.accentColor)
            Text("You're all set").font(.title.bold())
            Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 8) {
                ForEach(Self.essentials, id: \.title) { row in
                    GridRow {
                        Text(row.ids.compactMap { model.combos[$0] }.joined(separator: "  "))
                            .font(.body.monospaced())
                            .gridColumnAlignment(.trailing)
                        Text(row.title)
                    }
                }
            }
            Text("Every shortcut is listed in the menu bar under Keyboard Shortcuts, and can be changed in Settings.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            Toggle("Launch PaperWM at login", isOn: Binding(
                get: { launchAtLogin },
                set: { on in
                    SettingsView.setLaunchAtLogin(on)
                    launchAtLogin = SettingsView.isLaunchAtLogin
                }
            ))
        }
    }

    private struct Essential {
        /// `HotKey.id`s shown side by side.
        let ids: [String]
        let title: String
    }

    private static let essentials = [
        Essential(ids: ["focus-left", "focus-right"], title: "Focus the column on the left / right"),
        Essential(ids: ["move-left", "move-right"], title: "Move the column left / right"),
        Essential(ids: ["cycle-width"], title: "Cycle the column's width"),
        Essential(ids: ["switch-workspace-1"], title: "Switch to workspace 1 (and so on up to 9)"),
        Essential(ids: ["switcher"], title: "Switch windows"),
    ]

    private func feature(_ symbol: String, _ text: String) -> some View {
        Label {
            Text(text).fixedSize(horizontal: false, vertical: true)
        } icon: {
            Image(systemName: symbol).foregroundStyle(Color.accentColor).frame(width: 24)
        }
    }

    private func permission(symbol: String, title: String, text: String, granted: Bool, hint: String,
                            grant: @escaping () -> Void) -> some View {
        VStack(spacing: 16) {
            Image(systemName: symbol).font(.system(size: 44)).foregroundStyle(Color.accentColor)
            Text(title).font(.title.bold())
            Text(text)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
            if granted {
                Label("Access granted", systemImage: "checkmark.circle.fill")
                    .foregroundStyle(.green)
                    .font(.headline)
            } else {
                Button("Open System Settings", action: grant).controlSize(.large)
                Text(hint)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
        }
    }

    /// `anchor` names a pane of System Settings › Privacy & Security.
    private static func openPrivacySettings(_ anchor: String) {
        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?\(anchor)") {
            NSWorkspace.shared.open(url)
        }
    }
}

/// Owns the welcome guide's window so it can be reopened from the menu.
@MainActor
final class OnboardingWindowController: NSObject, NSWindowDelegate {
    private var window: NSWindow?
    private var timer: Timer?
    private let model = OnboardingModel()

    /// Without `step`, an open guide stays where it is and a closed one starts over.
    func show(at step: OnboardingModel.Step? = nil) {
        if let step {
            model.step = step
        } else if window?.isVisible != true {
            model.step = .welcome
        }
        // Read from the stored settings, which also works before the window manager starts.
        model.combos = Dictionary(Settings().hotKeys.map { ($0.id, $0.combo.symbol) }, uniquingKeysWith: { a, _ in a })
        refresh()
        if window == nil {
            let view = OnboardingView(model: model) { [weak self] in self?.window?.close() }
            let window = NSWindow(contentViewController: NSHostingController(rootView: view))
            window.title = "Welcome to PaperWM"
            window.styleMask = [.titled, .closable]
            window.isReleasedWhenClosed = false
            window.delegate = self
            self.window = window
        }
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        }
        window?.center()
        window?.present()
    }

    func windowWillClose(_ notification: Notification) {
        timer?.invalidate()
        timer = nil
    }

    private func refresh() {
        let wasTrusted = model.trusted
        let trusted = Engine.isTrusted()
        if trusted != model.trusted { model.trusted = trusted }
        let screenRecording = Thumbnails.hasAccess
        if screenRecording != model.screenRecording { model.screenRecording = screenRecording }
        // The user is in System Settings; bring them back to carry on.
        if trusted, !wasTrusted, model.step == .accessibility, window?.isVisible == true { window?.present() }
    }
}
