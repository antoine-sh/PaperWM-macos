import AppKit
import SwiftUI

/// PaperWM's minimap, as a small toolbar of app icons along the bottom of the
/// screen, shown briefly while navigating.
@MainActor
final class Minimap {
    private let panel: NSPanel
    private var hideTimer: Timer?

    init() {
        panel = NSPanel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered,
                        defer: true)
        panel.level = .statusBar
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.ignoresMouseEvents = true
        panel.collectionBehavior = [.canJoinAllSpaces, .transient]
    }

    /// Shows the active monitor's workspace; hides after a second unless
    /// shown again.
    func show(_ state: EngineState) {
        guard state.monitors.indices.contains(state.activeMonitor),
              NSScreen.screens.indices.contains(state.activeMonitor) else { return }
        let screen = NSScreen.screens[state.activeMonitor]
        let monitor = state.monitors[state.activeMonitor]
        let view = MinimapView(state: state, monitor: monitor, width: screen.visibleFrame.width * 0.8)
        let hosting = NSHostingView(rootView: view)
        panel.contentView = hosting
        let size = hosting.fittingSize
        let visible = screen.visibleFrame
        panel.setFrame(CGRect(x: visible.midX - size.width / 2, y: visible.minY + 12,
                              width: size.width, height: size.height), display: true)
        panel.orderFrontRegardless()

        hideTimer?.invalidate()
        hideTimer = Timer.scheduledTimer(withTimeInterval: 1, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated { self?.hide() }
        }
    }

    func hide() {
        hideTimer?.invalidate()
        hideTimer = nil
        panel.orderOut(nil)
    }
}

/// A row of app icons, one per column in strip order. The focused column is
/// highlighted; columns scrolled out of view are dimmed.
private struct MinimapView: View {
    let state: EngineState
    let monitor: EngineState.Monitor
    /// Widest the toolbar may grow.
    let width: CGFloat

    var body: some View {
        let workspace = state.workspaces[monitor.workspace]
        let count = CGFloat(max(workspace.columns.count, 1))
        let iconSize = min(36, max(16, (width - 16) / count - 6))

        HStack(spacing: 6) {
            if workspace.columns.isEmpty {
                Text(workspace.name).font(.caption).foregroundStyle(.secondary).padding(.horizontal, 6)
            }
            ForEach(Array(workspace.columns.enumerated()), id: \.offset) { index, column in
                icon(column, size: iconSize)
                    .padding(4)
                    .background(index == workspace.focusedColumn ? Color.accentColor.opacity(0.6) : .clear,
                                in: RoundedRectangle(cornerRadius: 8))
                    .opacity(isVisible(index, in: workspace) ? 1 : 0.45)
            }
        }
        .padding(6)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 12))
        .environment(\.colorScheme, .dark)
    }

    /// The column's focused window's app icon, with a badge when windows are
    /// stacked.
    private func icon(_ column: EngineState.Column, size: CGFloat) -> some View {
        let row = column.windows.indices.contains(column.focusedRow) ? column.focusedRow : 0
        let window = column.windows.isEmpty ? nil : state.window(column.windows[row])
        return Group {
            if let pid = window?.pid, let image = NSRunningApplication(processIdentifier: pid)?.icon {
                Image(nsImage: image).resizable()
            } else {
                Image(systemName: "macwindow").resizable().padding(size / 6)
            }
        }
        .aspectRatio(contentMode: .fit)
        .frame(width: size, height: size)
        .overlay(alignment: .topTrailing) {
            if column.windows.count > 1 {
                Text("\(column.windows.count)")
                    .font(.system(size: 9, weight: .bold))
                    .padding(.horizontal, 3)
                    .background(.red, in: Capsule())
                    .offset(x: 3, y: -3)
            }
        }
    }

    private func isVisible(_ index: Int, in workspace: EngineState.Workspace) -> Bool {
        let x = workspace.columns[..<index].reduce(0) { $0 + $1.width + state.gap }
        let viewport = max(monitor.area.width - 2 * state.gap, 1)
        return x + workspace.columns[index].width > workspace.scroll && x < workspace.scroll + viewport
    }
}
