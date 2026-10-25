import AppKit
import CPaperWMEngine

/// A window as reported by the engine, in Accessibility coordinates
/// (top-left origin of the primary display, y grows downwards).
struct ManagedWindow {
    let id: UInt32
    let pid: Int32
    let appName: String
    let title: String
    let frame: CGRect
    let isStandard: Bool
    let isMinimized: Bool
}

enum EngineError: Error, CustomStringConvertible {
    case notTrusted
    case windowNotFound
    case platform
    case nullPointer
    case unknown(UInt32)

    /// Nil when `status` is success.
    init?(_ status: PwmStatus) {
        switch status {
        case PWM_STATUS_OK: return nil
        case PWM_STATUS_NOT_TRUSTED: self = .notTrusted
        case PWM_STATUS_WINDOW_NOT_FOUND: self = .windowNotFound
        case PWM_STATUS_PLATFORM_ERROR: self = .platform
        case PWM_STATUS_NULL_POINTER: self = .nullPointer
        default: self = .unknown(status.rawValue)
        }
    }

    var description: String {
        switch self {
        case .notTrusted: "PaperWM needs Accessibility permission to manage windows."
        case .windowNotFound: "The window no longer exists."
        case .platform: "macOS refused the request."
        case .nullPointer: "The engine is not running."
        case .unknown(let code): "Unexpected engine error \(code)."
        }
    }
}

/// Owns the Rust engine. Use from the main thread only.
@MainActor
final class Engine {
    private let handle: OpaquePointer

    init() {
        handle = pwm_engine_new()
    }

    deinit {
        pwm_engine_free(handle)
    }

    static var version: String { String(cString: pwm_version()) }

    static func isTrusted(prompt: Bool = false) -> Bool {
        pwm_accessibility_is_trusted(prompt)
    }

    func windows() throws -> [ManagedWindow] {
        let list = pwm_engine_list_windows(handle)
        defer { pwm_window_list_free(list) }
        if let error = EngineError(list.status) { throw error }
        return UnsafeBufferPointer(start: list.windows, count: list.len).map { w in
            ManagedWindow(
                id: w.id,
                pid: w.pid,
                appName: String(cString: w.app_name),
                title: String(cString: w.title),
                frame: CGRect(w.frame),
                isStandard: w.is_standard,
                isMinimized: w.is_minimized
            )
        }
    }

    func setFrame(of windowID: UInt32, to frame: CGRect) throws {
        if let error = EngineError(pwm_engine_set_window_frame(handle, windowID, PwmRect(frame))) { throw error }
    }

    /// Tiles the windows on `area` into equal columns and returns how many were tiled.
    func tileColumns(in area: CGRect, gap: Double) throws -> Int {
        var tiled = 0
        if let error = EngineError(pwm_engine_tile_columns(handle, PwmRect(area), gap, &tiled)) { throw error }
        return tiled
    }
}

extension PwmRect {
    init(_ rect: CGRect) {
        self.init(x: rect.minX, y: rect.minY, width: rect.width, height: rect.height)
    }
}

extension CGRect {
    init(_ rect: PwmRect) {
        self.init(x: rect.x, y: rect.y, width: rect.width, height: rect.height)
    }
}

extension NSScreen {
    /// `visibleFrame` converted to Accessibility coordinates.
    var accessibilityVisibleFrame: CGRect {
        let primaryHeight = NSScreen.screens.first?.frame.height ?? frame.height
        let visible = visibleFrame
        return CGRect(x: visible.minX, y: primaryHeight - visible.maxY, width: visible.width, height: visible.height)
    }
}
