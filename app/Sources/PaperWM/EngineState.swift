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
    }

    struct Column: Decodable {
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

    /// Index of the workspace on the active monitor.
    var activeWorkspace: Int? {
        monitors.indices.contains(activeMonitor) ? monitors[activeMonitor].workspace : nil
    }

    func window(_ id: UInt32) -> Window? {
        windows.first { $0.id == id }
    }
}
