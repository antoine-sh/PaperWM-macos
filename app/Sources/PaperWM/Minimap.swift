import AppKit
import SwiftUI

/// PaperWM's minimap: a brief overview of the active workspace's strip,
/// shown while navigating.
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
        let view = MinimapView(state: state, monitor: monitor, width: screen.visibleFrame.width * 0.6)
        let hosting = NSHostingView(rootView: view)
        panel.contentView = hosting
        let size = hosting.fittingSize
        let visible = screen.visibleFrame
        panel.setFrame(CGRect(x: visible.midX - size.width / 2, y: visible.maxY - visible.height / 3 - size.height / 2,
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

private struct MinimapView: View {
    let state: EngineState
    let monitor: EngineState.Monitor
    /// Width available for the strip.
    let width: CGFloat

    private let stripHeight: CGFloat = 110

    var body: some View {
        let workspace = state.workspaces[monitor.workspace]
        let viewport = max(monitor.area.width - 2 * state.gap, 1)
        let stripWidth = workspace.columns.reduce(0) { $0 + $1.width + state.gap } - state.gap
        let scale = width / max(stripWidth, viewport)

        VStack(alignment: .leading, spacing: 8) {
            Text(workspace.name).font(.headline)
            ZStack(alignment: .topLeading) {
                ForEach(Array(workspace.columns.enumerated()), id: \.offset) { index, column in
                    let x = workspace.columns[..<index].reduce(0) { $0 + $1.width + state.gap }
                    columnView(column, focused: index == workspace.focusedColumn, scale: scale)
                        .frame(width: column.width * scale, height: stripHeight)
                        .offset(x: x * scale)
                }
                RoundedRectangle(cornerRadius: 6)
                    .stroke(Color.white.opacity(0.8), lineWidth: 2)
                    .frame(width: viewport * scale, height: stripHeight + 8)
                    .offset(x: workspace.scroll * scale, y: -4)
            }
            .frame(width: width, height: stripHeight, alignment: .topLeading)
            if workspace.columns.isEmpty {
                Text("Empty").foregroundStyle(.secondary)
            }
        }
        .padding(16)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 14))
        .environment(\.colorScheme, .dark)
    }

    private func columnView(_ column: EngineState.Column, focused: Bool, scale: CGFloat) -> some View {
        VStack(spacing: state.gap * scale) {
            ForEach(Array(column.windows.enumerated()), id: \.offset) { row, id in
                let isFocused = focused && row == column.focusedRow
                RoundedRectangle(cornerRadius: 4)
                    .fill(isFocused ? Color.accentColor : Color.white.opacity(0.2))
                    .overlay(
                        Text(state.window(id)?.appName ?? "")
                            .font(.caption2)
                            .lineLimit(1)
                            .padding(2)
                    )
            }
        }
    }
}
