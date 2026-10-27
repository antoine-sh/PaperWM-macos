import AppKit
import SwiftUI

/// PaperWM's Super+Tab: hold ⌃⌥ (or the modifiers of the rebound shortcut),
/// press Tab to step through windows in most recently used order, let go to
/// focus the selection.
@MainActor
final class Switcher {
    private let panel: NSPanel
    private let model = SwitcherModel()
    private var thumbnailTask: Task<Void, Never>?

    var isOpen: Bool { panel.isVisible }

    init() {
        panel = NSPanel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered,
                        defer: true)
        panel.level = .popUpMenu
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.ignoresMouseEvents = true
        panel.collectionBehavior = [.canJoinAllSpaces, .transient]
    }

    /// Opens the switcher on the previous window, or moves the selection.
    func step(forward: Bool, state: EngineState) {
        if !isOpen {
            let windows = state.mru.compactMap(state.window)
            guard !windows.isEmpty else { return }
            model.windows = windows
            model.thumbnails = [:]
            model.selection = windows.count > 1 ? (forward ? 1 : windows.count - 1) : 0
            open(on: state.activeMonitor)
        } else {
            let count = model.windows.count
            model.selection = (model.selection + (forward ? 1 : count - 1)) % count
        }
    }

    /// Closes the switcher and returns the chosen window.
    func commit() -> UInt32? {
        guard isOpen else { return nil }
        let id = model.windows.indices.contains(model.selection) ? model.windows[model.selection].id : nil
        close()
        return id
    }

    func cancel() {
        close()
    }

    private func open(on monitor: Int) {
        let screen = NSScreen.screens.indices.contains(monitor) ? NSScreen.screens[monitor] : NSScreen.main
        guard let visible = screen?.visibleFrame else { return }
        model.maxWidth = visible.width * 0.9
        // A fresh view sizes itself for the current windows right away.
        let hosting = NSHostingView(rootView: SwitcherView(model: model))
        panel.contentView = hosting
        let size = hosting.fittingSize
        panel.setFrame(CGRect(x: visible.midX - size.width / 2, y: visible.midY - size.height / 2,
                              width: size.width, height: size.height), display: true)
        panel.orderFrontRegardless()

        Thumbnails.askForAccessOnce()
        let ids = model.windows.map(\.id)
        thumbnailTask = Task { [model] in
            await Thumbnails.capture(ids: ids, maxSize: SwitcherView.cardSize.width) { id, image in
                if !Task.isCancelled { model.thumbnails[id] = image }
            }
        }
    }

    private func close() {
        thumbnailTask?.cancel()
        thumbnailTask = nil
        panel.orderOut(nil)
    }
}

@MainActor
private final class SwitcherModel: ObservableObject {
    @Published var windows: [EngineState.Window] = []
    @Published var selection = 0
    @Published var thumbnails: [UInt32: CGImage] = [:]
    @Published var maxWidth: CGFloat = 800
}

private struct SwitcherView: View {
    @ObservedObject var model: SwitcherModel

    static let cardSize = CGSize(width: 200, height: 130)

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 12) {
                    ForEach(Array(model.windows.enumerated()), id: \.element.id) { index, window in
                        card(window, selected: index == model.selection).id(index)
                    }
                }
                .padding(16)
            }
            .onChange(of: model.selection) { _, selection in proxy.scrollTo(selection) }
        }
        .frame(width: min(model.maxWidth, CGFloat(model.windows.count) * (Self.cardSize.width + 28) + 20),
               height: Self.cardSize.height + 70)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 16))
        .environment(\.colorScheme, .dark)
    }

    private func card(_ window: EngineState.Window, selected: Bool) -> some View {
        VStack(spacing: 6) {
            Group {
                if let image = model.thumbnails[window.id] {
                    Image(decorative: image, scale: 2).resizable().aspectRatio(contentMode: .fit)
                } else if let icon = AppIcons.icon(for: window.pid) {
                    Image(nsImage: icon).resizable().aspectRatio(contentMode: .fit).padding(24)
                } else {
                    Image(systemName: "macwindow").resizable().aspectRatio(contentMode: .fit).padding(36)
                }
            }
            .frame(width: Self.cardSize.width, height: Self.cardSize.height)
            Text(window.title.isEmpty ? window.appName : window.title)
                .font(.caption)
                .lineLimit(1)
                .frame(width: Self.cardSize.width)
        }
        .padding(6)
        .background(selected ? Color.accentColor.opacity(0.6) : .clear, in: RoundedRectangle(cornerRadius: 10))
    }
}
