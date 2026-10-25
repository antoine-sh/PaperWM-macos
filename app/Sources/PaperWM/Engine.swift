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
    case invalidArgument
    case unknown(UInt32)

    /// Nil when `status` is success.
    init?(_ status: PwmStatus) {
        switch status {
        case PWM_STATUS_OK: return nil
        case PWM_STATUS_NOT_TRUSTED: self = .notTrusted
        case PWM_STATUS_WINDOW_NOT_FOUND: self = .windowNotFound
        case PWM_STATUS_PLATFORM_ERROR: self = .platform
        case PWM_STATUS_NULL_POINTER: self = .nullPointer
        case PWM_STATUS_INVALID_ARGUMENT: self = .invalidArgument
        default: self = .unknown(status.rawValue)
        }
    }

    var description: String {
        switch self {
        case .notTrusted: "PaperWM needs Accessibility permission to manage windows."
        case .windowNotFound: "The window no longer exists."
        case .platform: "macOS refused the request."
        case .nullPointer: "The engine is not running."
        case .invalidArgument: "The engine rejected an invalid request."
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

    /// Usable area of each monitor in Accessibility coordinates, main one first.
    func setMonitors(_ areas: [CGRect]) throws {
        let rects = areas.map(PwmRect.init)
        let status = rects.withUnsafeBufferPointer { pwm_engine_set_monitors(handle, $0.baseAddress, $0.count) }
        if let error = EngineError(status) { throw error }
    }

    /// Brings the layout up to date with the current windows and focus.
    func sync() throws {
        if let error = EngineError(pwm_engine_sync(handle)) { throw error }
    }

    func run(_ command: PwmCommand, arg: Double = 0) throws {
        if let error = EngineError(pwm_engine_command(handle, command.rawValue, arg)) { throw error }
    }

    func observeApp(_ pid: pid_t) {
        pwm_engine_observe_app(handle, pid)
    }

    func forgetApp(_ pid: pid_t) {
        pwm_engine_forget_app(handle, pid)
    }

    /// `handler(context)` runs on the main run loop when observed windows
    /// change. It must not call into the engine directly.
    func setChangeHandler(
        _ handler: @convention(c) (UnsafeMutableRawPointer?) -> Void,
        context: UnsafeMutableRawPointer
    ) {
        pwm_engine_set_change_handler(handle, handler, context)
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
