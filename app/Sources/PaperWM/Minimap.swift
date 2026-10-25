import AppKit
import SwiftUI

/// PaperWM's minimap: a brief overview of the active workspace's strip,
/// shown while navigating, either as the strip itself in the middle of the
/// screen or as a toolbar of app icons along the bottom.
@MainActor
final class Minimap {
    private let panel: NSPanel
    private var hideTimer: Timer?
    /// Kept between showings so SwiftUI updates the views instead of building
    /// them again every time.
    private var strip: NSHostingView<StripView>?
    private var toolbar: NSHostingView<ToolbarView>?

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
    func show(_ state: EngineState, style: MinimapStyle) {
        guard state.monitors.indices.contains(state.activeMonitor),
              NSScreen.screens.indices.contains(state.activeMonitor) else { return }
        let screen = NSScreen.screens[state.activeMonitor]
        let monitor = state.monitors[state.activeMonitor]
        let visible = screen.visibleFrame
        let hosting: NSView
        switch style {
        case .strip:
            let view = StripView(state: state, monitor: monitor, width: visible.width * 0.6)
            if let strip { strip.rootView = view } else { strip = NSHostingView(rootView: view) }
            hosting = strip!
        case .toolbar:
            let view = ToolbarView(state: state, monitor: monitor, width: visible.width * 0.8)
            if let toolbar { toolbar.rootView = view } else { toolbar = NSHostingView(rootView: view) }
            hosting = toolbar!
        }
        if panel.contentView !== hosting { panel.contentView = hosting }
        let size = hosting.fittingSize
        let y = switch style {
        case .strip: visible.maxY - visible.height / 3 - size.height / 2
        case .toolbar: visible.minY + 12
        }
        panel.setFrame(CGRect(x: visible.midX - size.width / 2, y: y, width: size.width, height: size.height),
                       display: true)
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

/// The strip overview: each window as a box, with the viewport outlined.
struct StripView: View {
    let state: EngineState
    let monitor: EngineState.Monitor
    /// Width available for the strip.
    let width: CGFloat

    private let stripHeight: CGFloat = 110

    var body: some View {
        let workspace = state.workspaces[monitor.workspace]
        let viewport = monitor.viewport
        let stripWidth = workspace.columns.last.map { $0.x + $0.width } ?? 0
        let scale = width / max(stripWidth, viewport)

        VStack(alignment: .leading, spacing: 8) {
            Text(workspace.name).font(.headline)
            ZStack(alignment: .topLeading) {
                ForEach(Array(workspace.columns.enumerated()), id: \.offset) { index, column in
                    columnView(column, focused: index == workspace.focusedColumn, scale: scale)
                        .frame(width: column.width * scale, height: stripHeight)
                        .offset(x: column.x * scale)
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

/// A row of app icons, one per column in strip order. The focused column is
/// highlighted; columns scrolled out of view are dimmed.
struct ToolbarView: View {
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
            if let pid = window?.pid, let image = AppIcons.icon(for: pid) {
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
        let column = workspace.columns[index]
        return column.x + column.width > workspace.scroll && column.x < workspace.scroll + monitor.viewport
    }
}
