import AppKit
import SwiftUI

/// A small bar of tabs along the bottom of each tabbed column on screen, one
/// per window; clicking a tab focuses its window. The bottom edge keeps the
/// active window's title bar and traffic lights uncovered.
@MainActor
final class TabBars {
    private var panels: [TabPanel] = []
    private let focus: (UInt32) -> Void
    private static let height: CGFloat = 24

    init(focus: @escaping (UInt32) -> Void) {
        self.focus = focus
    }

    /// Shows a bar for every tabbed column fully on screen in `state`.
    // shortcut: bars jump to where a sliding or panned strip is headed; follow
    // the animated scroll if that looks wrong in practice.
    func update(_ state: EngineState) {
        var bars: [(frame: CGRect, view: TabBarView)] = []
        for monitor in state.monitors where state.workspaces.indices.contains(monitor.workspace) {
            let workspace = state.workspaces[monitor.workspace]
            let area = monitor.area.rect
            for column in workspace.columns where column.tabbed && !column.windows.isEmpty {
                let x = area.minX + state.gap + column.x - workspace.scroll
                // Parked columns sit against the screen edge; a bar there would float on its own.
                guard x >= area.minX, x + column.width <= area.maxX else { continue }
                let frame = CGRect(x: x, y: area.maxY - state.gap - Self.height, width: column.width,
                                   height: Self.height)
                let tabs = column.windows.map { id in
                    let title = state.window(id).map { $0.title.isEmpty ? $0.appName : $0.title }
                    return TabBarView.Tab(id: id, title: title ?? "")
                }
                let active = column.windows[min(max(column.focusedRow, 0), column.windows.count - 1)]
                bars.append((frame: frame.flippedForScreens, view: TabBarView(tabs: tabs, active: active)))
            }
        }
        while panels.count < bars.count {
            panels.append(Self.panel(focus: focus))
        }
        for (index, panel) in panels.enumerated() {
            guard index < bars.count else {
                panel.orderOut(nil)
                continue
            }
            let bar = bars[index]
            panel.tabWindows = bar.view.tabs.map(\.id)
            if let hosting = panel.contentView as? NSHostingView<TabBarView> {
                hosting.rootView = bar.view
            } else {
                panel.contentView = NSHostingView(rootView: bar.view)
            }
            if panel.frame != bar.frame { panel.setFrame(bar.frame, display: true) }
            if !panel.isVisible { panel.orderFrontRegardless() }
        }
    }

    private static func panel(focus: @escaping (UInt32) -> Void) -> TabPanel {
        let panel = TabPanel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered,
                             defer: true)
        panel.onPick = focus
        panel.level = .floating
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle, .fullScreenAuxiliary]
        return panel
    }
}

/// Picks the tab under a click itself: the panel never becomes key, so
/// SwiftUI buttons in it would swallow the first click.
private final class TabPanel: NSPanel {
    /// The column's windows, one equal-width tab each, left to right.
    var tabWindows: [UInt32] = []
    var onPick: ((UInt32) -> Void)?

    override func sendEvent(_ event: NSEvent) {
        guard event.type == .leftMouseDown, !tabWindows.isEmpty, frame.width > 0 else {
            super.sendEvent(event)
            return
        }
        let index = Int(event.locationInWindow.x / frame.width * CGFloat(tabWindows.count))
        onPick?(tabWindows[min(max(index, 0), tabWindows.count - 1)])
    }
}

/// The tabs of one column, the shown window highlighted.
struct TabBarView: View {
    struct Tab: Identifiable {
        let id: UInt32
        let title: String
    }

    let tabs: [Tab]
    let active: UInt32

    var body: some View {
        HStack(spacing: 2) {
            ForEach(tabs) { tab in
                Text(tab.title)
                    .font(.caption)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .padding(.horizontal, 6)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(tab.id == active ? Color.accentColor.opacity(0.7) : Color.white.opacity(0.12),
                                in: RoundedRectangle(cornerRadius: 5))
            }
        }
        .padding(2)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 7))
        .environment(\.colorScheme, .dark)
    }
}
