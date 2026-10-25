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
    /// macOS refused or did not answer, with the engine's explanation.
    case platform(String)
    case nullPointer
    case invalidArgument
    case unknown(UInt32)

    /// Nil when `status` is success.
    init?(_ status: PwmStatus) {
        switch status {
        case PWM_STATUS_OK: return nil
        case PWM_STATUS_NOT_TRUSTED: self = .notTrusted
        case PWM_STATUS_WINDOW_NOT_FOUND: self = .windowNotFound
        case PWM_STATUS_PLATFORM_ERROR: self = .platform(String(cString: pwm_last_error()))
        case PWM_STATUS_NULL_POINTER: self = .nullPointer
        case PWM_STATUS_INVALID_ARGUMENT: self = .invalidArgument
        default: self = .unknown(status.rawValue)
        }
    }

    var description: String {
        switch self {
        case .notTrusted: "PaperWM needs Accessibility permission to manage windows."
        case .windowNotFound: "The window no longer exists."
        case .platform(let message): "macOS refused the request: \(message)"
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

    /// Puts every window back in its place and syncs.
    func retile() throws {
        if let error = EngineError(pwm_engine_retile(handle)) { throw error }
    }

    /// Runs `command`. Returns whether it changed anything.
    @discardableResult
    func run(_ command: PwmCommand, arg: Double = 0) throws -> Bool {
        var changed = false
        if let error = EngineError(pwm_engine_command(handle, command.rawValue, arg, &changed)) { throw error }
        return changed
    }

    /// Advances strip slides by `seconds`. Returns whether one is still sliding.
    func tick(_ seconds: Double) throws -> Bool {
        var status = PWM_STATUS_OK
        let sliding = pwm_engine_tick(handle, seconds, &status)
        if let error = EngineError(status) { throw error }
        return sliding
    }

    /// Whether a strip is sliding, so `tick` should run every frame.
    var isAnimating: Bool { pwm_engine_is_animating(handle) }

    private static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }()

    /// The current layout, or nil if the engine could not describe it.
    func state() -> EngineState? {
        guard let json = pwm_engine_state_json(handle) else { return nil }
        defer { pwm_string_free(json) }
        // Decode the engine's buffer in place rather than copying it twice.
        let data = Data(bytesNoCopy: UnsafeMutableRawPointer(json), count: strlen(json), deallocator: .none)
        guard var state = try? Self.decoder.decode(EngineState.self, from: data) else { return nil }
        state.indexWindows()
        return state
    }

    /// Applies settings given as `engine::Config` JSON.
    func setConfig(json: String) throws {
        let status = json.withCString { pwm_engine_set_config(handle, $0) }
        if let error = EngineError(status) { throw error }
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

extension CGRect {
    /// Converts between Accessibility coordinates (top-left origin, y down)
    /// and Cocoa coordinates (bottom-left origin, y up); the flip is the
    /// same both ways.
    var flippedForScreens: CGRect {
        let primaryHeight = NSScreen.screens.first?.frame.height ?? 0
        return CGRect(x: minX, y: primaryHeight - maxY, width: width, height: height)
    }
}

extension NSScreen {
    /// `visibleFrame` converted to Accessibility coordinates.
    var accessibilityVisibleFrame: CGRect { visibleFrame.flippedForScreens }
}
