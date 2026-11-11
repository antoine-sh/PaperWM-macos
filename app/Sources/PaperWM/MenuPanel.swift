import AppKit
import CPaperWMEngine
import SwiftUI

/// What the menu bar panel asks the app to do.
enum PanelAction {
    case setWidth(Double)
    case arrange(PanelLayout)
    case focus(UInt32)
    case setEnabled(Bool)
    case settings
    case retile
    case guide
    case quit
}

/// A way to lay out the active strip: columns left to right, each with a
/// number of stacked windows and a share of the screen width.
struct PanelLayout: Identifiable, Equatable {
    struct Column: Equatable {
        let rows: Int
        let fraction: Double
    }

    let columns: [Column]
    let summary: String
    var id: String { columns.map { "\($0.rows)@\($0.fraction)" }.joined(separator: ",") }

    /// Layouts for `count` windows: even grids of one to four columns, a main
    /// window beside the rest, and for many windows the plain scrolling strip.
    static func suggestions(for count: Int) -> [PanelLayout] {
        guard count > 0 else { return [] }
        let name = Self.number(count)
        var layouts: [PanelLayout] = []
        func add(_ columns: [Column], _ summary: String) {
            let layout = PanelLayout(columns: columns, summary: summary)
            if !layouts.contains(where: { $0.columns == columns }) { layouts.append(layout) }
        }
        for c in 1...min(count, 4) {
            // Extra windows stack in the rightmost columns.
            let columns = (0..<c).map { i in
                Column(rows: count / c + (i >= c - count % c ? 1 : 0), fraction: 1 / Double(c))
            }
            let summary = switch (count, c) {
            case (1, _): "One window, full width."
            case (_, 1): "\(name) windows stacked in one full-width column."
            case _ where c == count: "\(name) windows side by side, sharing the width."
            default: "\(name) windows in \(Self.number(c).lowercased()) equal columns, stacked on the right."
            }
            add(columns, summary)
        }
        if count >= 2 {
            add([Column(rows: 1, fraction: 2 / 3), Column(rows: count - 1, fraction: 1 / 3)],
                "One main window at two thirds, the rest stacked beside it.")
        }
        if count >= 3 {
            let rest = count - 1
            add([Column(rows: 1, fraction: 1 / 2), Column(rows: rest / 2, fraction: 1 / 4),
                 Column(rows: rest - rest / 2, fraction: 1 / 4)],
                "One main window at half width, the rest in two narrow columns.")
        }
        if count > 4 {
            add(Array(repeating: Column(rows: 1, fraction: 1 / 3), count: count),
                "\(name) windows a third wide each; the strip scrolls.")
        }
        return Array(layouts.prefix(8))
    }

    private static func number(_ n: Int) -> String {
        let words = ["Zero", "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine", "Ten"]
        return n < words.count ? words[n] : "\(n)"
    }
}

/// Everything the panel shows, kept current while it is open.
@MainActor
final class PanelModel: ObservableObject {
    @Published var state: EngineState?
    @Published var enabled = true
    @Published var presets: [Double] = []
    @Published var hovered: String?
    var perform: (PanelAction) -> Void = { _ in }

    private var monitor: EngineState.Monitor? {
        guard let state, state.monitors.indices.contains(state.activeMonitor) else { return nil }
        return state.monitors[state.activeMonitor]
    }

    private var workspace: EngineState.Workspace? {
        guard let state, let monitor, state.workspaces.indices.contains(monitor.workspace) else { return nil }
        return state.workspaces[monitor.workspace]
    }

    /// The active strip's unpinned columns, which layouts rearrange.
    var columns: [EngineState.Column] { workspace?.columns.filter { $0.pinned == nil } ?? [] }

    var windows: [UInt32] { columns.flatMap(\.windows) }

    /// The active strip's focused window, which the width buttons resize; macOS
    /// focus can be on a floating window or another display.
    var focused: UInt32? {
        guard let workspace, workspace.columns.indices.contains(workspace.focusedColumn) else { return nil }
        let column = workspace.columns[workspace.focusedColumn]
        return column.windows.indices.contains(column.focusedRow) ? column.windows[column.focusedRow] : nil
    }

    func pid(of id: UInt32) -> Int32? { state?.window(id)?.pid }

    /// Share of the screen `width` takes, the inverse of the engine's preset sizing.
    func fraction(_ width: Double) -> Double {
        guard let state, let monitor else { return 0 }
        return (width + state.gap) / (monitor.viewport + state.gap)
    }

    var focusedFraction: Double? {
        guard let workspace, workspace.columns.indices.contains(workspace.focusedColumn) else { return nil }
        return fraction(workspace.columns[workspace.focusedColumn].width)
    }

    var current: [PanelLayout.Column] {
        columns.map { PanelLayout.Column(rows: $0.windows.count, fraction: fraction($0.width)) }
    }

    func isCurrent(_ layout: PanelLayout) -> Bool {
        let current = current
        return current.count == layout.columns.count && zip(current, layout.columns).allSatisfy {
            $0.rows == $1.rows && abs($0.fraction - $1.fraction) < 0.02
        }
    }

    var footer: String {
        if !enabled { return "Tiling is off: windows stay where you put them." }
        if let hovered { return hovered }
        let n = windows.count
        return n == 0 ? "No windows to arrange on this screen." : "Pick a layout for the \(n) windows on this screen."
    }
}

struct MenuPanelView: View {
    @ObservedObject var model: PanelModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .top, spacing: 16) {
                widths
                Divider()
                layouts
                Divider()
                controls
            }
            .padding(16)
            Divider()
            Text(model.footer)
                .font(.callout)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
        }
        // 110 widths + 4 × 78 layouts with 3 × 8 gaps + 32 controls + 4 × 16 spacing + 2 dividers + 2 × 16 padding.
        .frame(width: 576)
    }

    private var widths: some View {
        VStack(alignment: .leading, spacing: 8) {
            PanelHeading(title: "Layout options")
            ForEach(model.presets, id: \.self) { fraction in
                let selected = model.focusedFraction.map { abs($0 - fraction) < 0.02 } ?? false
                Button { model.perform(.setWidth(fraction)) } label: {
                    GeometryReader { geo in
                        ZStack(alignment: .leading) {
                            RoundedRectangle(cornerRadius: 4).fill(Color.primary.opacity(0.08))
                            Cell(pid: model.focused.flatMap(model.pid(of:)), highlighted: true)
                                .frame(width: geo.size.width * fraction)
                        }
                    }
                    .padding(4)
                    .frame(width: 110, height: 38)
                    .tile(selected: selected)
                }
                .buttonStyle(.plain)
                .help("Focused column at \(Self.percent(fraction)) of the screen")
                .onHover { inside in
                    model.hovered = inside ? "The focused column takes \(Self.percent(fraction)) of the screen." : nil
                }
            }
        }
        .disabled(!model.enabled || model.focused == nil)
        .opacity(model.enabled ? 1 : 0.4)
    }

    private var layouts: some View {
        let windows = model.windows
        return VStack(alignment: .leading, spacing: 8) {
            PanelHeading(title: "Layouts")
            // Many windows scroll sideways rather than widen the panel.
            ScrollView(.horizontal, showsIndicators: false) { HStack(spacing: 8) {
                ForEach(windows, id: \.self) { id in
                    Button { model.perform(.focus(id)) } label: {
                        AppIcon(pid: model.pid(of: id)).frame(width: 22, height: 22).padding(4)
                            .tile(selected: id == model.focused)
                    }
                    .buttonStyle(.plain)
                    .help(model.state?.window(id).map { $0.title.isEmpty ? $0.appName : $0.title } ?? "")
                }
            } }
            .frame(height: 30, alignment: .leading)
            LazyVGrid(columns: Array(repeating: GridItem(.fixed(78), spacing: 8), count: 4), spacing: 8) {
                ForEach(PanelLayout.suggestions(for: windows.count)) { layout in
                    Button { model.perform(.arrange(layout)) } label: {
                        Thumbnail(layout: layout, pids: windows.map(model.pid(of:)))
                            .padding(4)
                            .frame(width: 78, height: 52)
                            .tile(selected: model.isCurrent(layout))
                    }
                    .buttonStyle(.plain)
                    .help(layout.summary)
                    .onHover { model.hovered = $0 ? layout.summary : nil }
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .disabled(!model.enabled)
        .opacity(model.enabled ? 1 : 0.4)
    }

    private var controls: some View {
        VStack(spacing: 8) {
            // An invisible heading so the first button lines up with the other columns' first tiles.
            PanelHeading(title: " ").hidden().accessibilityHidden(true)
            control("gearshape", "Settings…", .settings)
            control("arrow.clockwise", "Re-tile Now", .retile).disabled(!model.enabled)
            control("power", "Quit PaperWM", .quit)
            Spacer(minLength: 12)
            control("questionmark.circle", "Welcome Guide…", .guide)
            Toggle("Tiling", isOn: Binding(get: { model.enabled }, set: { model.perform(.setEnabled($0)) }))
                .toggleStyle(.switch)
                .labelsHidden()
                .help(model.enabled ? "Turn tiling off" : "Turn tiling on")
        }
    }

    private func control(_ symbol: String, _ title: String, _ action: PanelAction) -> some View {
        Button { model.perform(action) } label: {
            Image(systemName: symbol)
                .font(.system(size: 14))
                .frame(width: 32, height: 32)
                .background(Circle().fill(Color.primary.opacity(0.08)))
        }
        .buttonStyle(.plain)
        .help(title)
        .accessibilityLabel(title)
    }

    private static func percent(_ fraction: Double) -> String {
        "\(Int((fraction * 100).rounded()))%"
    }
}

/// A layout drawn as its columns of stacked windows, each showing the app
/// that would land there.
private struct Thumbnail: View {
    let layout: PanelLayout
    let pids: [Int32?]

    var body: some View {
        GeometryReader { geo in
            // A strip wider than the screen is drawn squeezed to fit.
            let total = max(layout.columns.map(\.fraction).reduce(0, +), 1)
            let spacing: CGFloat = 2
            let usable = geo.size.width - spacing * CGFloat(layout.columns.count - 1)
            HStack(spacing: spacing) {
                ForEach(Array(layout.columns.enumerated()), id: \.offset) { index, column in
                    let first = layout.columns[..<index].map(\.rows).reduce(0, +)
                    VStack(spacing: spacing) {
                        ForEach(0..<column.rows, id: \.self) { row in
                            Cell(pid: first + row < pids.count ? pids[first + row] : nil, highlighted: false)
                        }
                    }
                    .frame(width: max(usable * column.fraction / total, 1))
                }
            }
        }
    }
}

/// One window in a thumbnail: a rounded box with its app's icon when it fits.
private struct Cell: View {
    let pid: Int32?
    let highlighted: Bool

    var body: some View {
        GeometryReader { geo in
            let side = min(geo.size.width, geo.size.height) * 0.6
            RoundedRectangle(cornerRadius: 3)
                .fill(highlighted ? Color.accentColor.opacity(0.35) : Color.primary.opacity(0.12))
                .overlay {
                    if side >= 8 { AppIcon(pid: pid).frame(width: min(side, 18), height: min(side, 18)) }
                }
        }
    }
}

private struct AppIcon: View {
    let pid: Int32?

    var body: some View {
        if let pid, let image = AppIcons.icon(for: pid) {
            Image(nsImage: image).resizable().aspectRatio(contentMode: .fit)
        } else {
            Image(systemName: "macwindow").resizable().aspectRatio(contentMode: .fit).foregroundStyle(.secondary)
        }
    }
}

/// The panel's section title, also used by the Settings window.
struct PanelHeading: View {
    let title: String

    var body: some View {
        Text(title.uppercased())
            .font(.caption.weight(.semibold))
            .kerning(0.8)
            .foregroundStyle(Color.accentColor)
    }
}

extension View {
    /// The panel's rounded tile, tinted when it is the current choice; the
    /// Settings window uses it too.
    func tile(selected: Bool) -> some View {
        background(
            RoundedRectangle(cornerRadius: 8)
                .fill(selected ? Color.accentColor.opacity(0.18) : Color.primary.opacity(0.05))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .strokeBorder(selected ? Color.accentColor.opacity(0.8) : Color.primary.opacity(0.08), lineWidth: 1)
        )
        .contentShape(RoundedRectangle(cornerRadius: 8))
    }
}
