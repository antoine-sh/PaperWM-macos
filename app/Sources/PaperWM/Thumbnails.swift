import AppKit
import ScreenCaptureKit

/// Window previews for the switcher. They need Screen Recording permission;
/// without it every capture comes back empty and the switcher shows icons.
enum Thumbnails {
    static var hasAccess: Bool { CGPreflightScreenCaptureAccess() }

    /// Captures the windows `ids`, at most `maxSize` points on their long
    /// side, all at once, handing each image over as soon as it is ready.
    static func capture(ids: [UInt32], maxSize: CGFloat,
                        onImage: @escaping @MainActor (UInt32, CGImage) -> Void) async {
        guard hasAccess,
              // Every managed window is on screen, if only by the sliver of a
              // parked one, so windows on other Spaces need not be listed.
              let content = try? await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
        else { return }
        let wanted = Set(ids)
        await withTaskGroup(of: (UInt32, CGImage?).self) { group in
            for window in content.windows where wanted.contains(window.windowID) {
                let frame = window.frame
                guard frame.width > 0, frame.height > 0 else { continue }
                group.addTask {
                    let scale = min(1, maxSize / max(frame.width, frame.height)) * 2
                    let configuration = SCStreamConfiguration()
                    configuration.width = Int(frame.width * scale)
                    configuration.height = Int(frame.height * scale)
                    configuration.showsCursor = false
                    let filter = SCContentFilter(desktopIndependentWindow: window)
                    let image = try? await SCScreenshotManager.captureImage(contentFilter: filter,
                                                                            configuration: configuration)
                    return (window.windowID, image)
                }
            }
            for await (id, image) in group {
                if Task.isCancelled {
                    group.cancelAll()
                    return
                }
                if let image { await onImage(id, image) }
            }
        }
    }
}

/// App icons by process, looked up once per app.
@MainActor
enum AppIcons {
    private static var icons: [pid_t: NSImage] = [:]

    static func icon(for pid: pid_t) -> NSImage? {
        if let icon = icons[pid] { return icon }
        guard let icon = NSRunningApplication(processIdentifier: pid)?.icon else { return nil }
        if icons.count > 128 { icons.removeAll() }
        icons[pid] = icon
        return icon
    }
}

/// Measures how round a window's corners are, which differs between apps and
/// macOS versions. Needs Screen Recording permission, like the previews.
enum WindowCorners {
    /// Side of the corner that is captured, in points.
    private static let cornerSize: CGFloat = 64

    /// The radius in points, or nil without permission or a usable image.
    static func radius(of id: UInt32) async -> CGFloat? {
        guard Thumbnails.hasAccess,
              let content = try? await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true),
              let window = content.windows.first(where: { $0.windowID == id }),
              window.frame.width > 0, window.frame.height > 0 else { return nil }
        // Only the top-left corner matters, so only it is captured.
        let side = min(cornerSize, window.frame.width, window.frame.height)
        let configuration = SCStreamConfiguration()
        configuration.sourceRect = CGRect(x: 0, y: 0, width: side, height: side)
        configuration.width = Int(side * 2)
        configuration.height = Int(side * 2)
        configuration.showsCursor = false
        configuration.ignoreShadowsSingleWindow = true
        let filter = SCContentFilter(desktopIndependentWindow: window)
        guard let image = try? await SCScreenshotManager.captureImage(contentFilter: filter,
                                                                    configuration: configuration) else { return nil }
        return radius(in: image, scale: CGFloat(image.width) / side)
    }

    /// Walks the top-left corner's diagonal to the first opaque pixel, which
    /// for a rounded corner of radius r lies r(1 - 1/√2) in on each axis.
    static func radius(in image: CGImage, scale: CGFloat) -> CGFloat? {
        let size = min(image.width, image.height, Int(64 * scale))
        guard size > 0, let corner = image.cropping(to: CGRect(x: 0, y: 0, width: size, height: size)),
              let context = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8,
                                      bytesPerRow: size * 4, space: CGColorSpaceCreateDeviceRGB(),
                                      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue),
              let data = context.data else { return nil }
        context.draw(corner, in: CGRect(x: 0, y: 0, width: size, height: size))
        // Row 0 of the buffer is the top of the image.
        let pixels = data.bindMemory(to: UInt8.self, capacity: size * size * 4)
        // An opaque corner pixel more likely means the capture lost its alpha
        // than a square window, so it doesn't count as a measurement.
        for d in 1..<max(size, 1) where pixels[(d * size + d) * 4 + 3] > 128 {
            return pixels[3] > 128 ? nil : CGFloat(d) / (1 - 1 / 2.0.squareRoot()) / scale
        }
        return nil
    }
}
