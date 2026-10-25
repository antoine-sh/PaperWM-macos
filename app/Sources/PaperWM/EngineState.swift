import CoreGraphics

/// The engine's layout snapshot (`engine::State`), decoded from JSON. The UI
/// draws everything from it.
struct EngineState: Decodable {
    struct Area: Decodable {
        let x, y, width, height: Double

        var rect: CGRect { CGRect(x: x, y: y, width: width, height: height) }
    }

    struct Monitor: Decodable {
        /// Usable area in Accessibility coordinates.
        let area: Area
        let workspace: Int
        /// Width of the strip's viewport: the area without the outer gaps.
        let viewport: Double
    }

    struct Column: Decodable {
        /// Strip x of the column's left edge.
        let x: Double
        let width: Double
        let windows: [UInt32]
        let focusedRow: Int
    }

    struct Workspace: Decodable {
        let name: String
        let scroll: Double
        let focusedColumn: Int
        let columns: [Column]
    }

    struct Window: Decodable {
        let id: UInt32
        let pid: Int32
        let appName: String
        let title: String
        /// Where the engine put the window, in Accessibility coordinates.
        let frame: Area
    }

    let activeMonitor: Int
    let gap: Double
    let monitors: [Monitor]
    let workspaces: [Workspace]
    let windows: [Window]
    /// Window ids, most recently focused first.
    let mru: [UInt32]
    let scratch: [UInt32]
    let scratchVisible: Bool
    /// The window with keyboard focus, managed or not, if the engine knows it.
    let focusedWindow: UInt32?
    /// `windows` by id, filled in by `indexWindows`.
    private var windowsByID: [UInt32: Window] = [:]

    private enum CodingKeys: String, CodingKey {
        case activeMonitor, gap, monitors, workspaces, windows, mru, scratch, scratchVisible, focusedWindow
    }

    /// Index of the workspace on the active monitor.
    var activeWorkspace: Int? {
        monitors.indices.contains(activeMonitor) ? monitors[activeMonitor].workspace : nil
    }

    mutating func indexWindows() {
        windowsByID = Dictionary(windows.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
    }

    func window(_ id: UInt32) -> Window? {
        windowsByID.isEmpty ? windows.first { $0.id == id } : windowsByID[id]
    }
}
