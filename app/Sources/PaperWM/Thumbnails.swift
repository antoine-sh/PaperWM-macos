import CoreGraphics
import ScreenCaptureKit

/// Window previews for the switcher. They need Screen Recording permission;
/// without it every capture comes back empty and the switcher shows icons.
enum Thumbnails {
    static var hasAccess: Bool { CGPreflightScreenCaptureAccess() }

    /// Images of the windows `ids`, at most `maxSize` points on their long side.
    static func capture(ids: [UInt32], maxSize: CGFloat) async -> [UInt32: CGImage] {
        guard hasAccess,
              let content = try? await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false)
        else { return [:] }
        var images: [UInt32: CGImage] = [:]
        for window in content.windows where ids.contains(window.windowID) {
            let frame = window.frame
            guard frame.width > 0, frame.height > 0 else { continue }
            let scale = min(1, maxSize / max(frame.width, frame.height)) * 2
            let configuration = SCStreamConfiguration()
            configuration.width = Int(frame.width * scale)
            configuration.height = Int(frame.height * scale)
            configuration.showsCursor = false
            let filter = SCContentFilter(desktopIndependentWindow: window)
            if let image = try? await SCScreenshotManager.captureImage(contentFilter: filter,
                                                                       configuration: configuration) {
                images[window.windowID] = image
            }
        }
        return images
    }
}
