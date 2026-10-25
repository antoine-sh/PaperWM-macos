import AppKit

/// Highlights the focused window: a colored border around it and a dim over
/// everything behind it. Both are click-through panels that follow the
/// frontmost window through the window server, so they also track windows
/// PaperWM doesn't tile and windows being dragged.
@MainActor
final class Decorations {
    private let border: NSPanel
    private let borderView = BorderView()
    /// One per screen, at normal window level just below the focused window.
    private var dims: [NSPanel] = []
    private var values = Settings.Values()
    private var timer: Timer?
    private var focusedID: CGWindowID?

    init() {
        border = Self.panel(level: .floating)
        border.contentView = borderView
    }

    func apply(_ values: Settings.Values) {
        self.values = values
        borderView.color = NSColor(hex: values.borderColor) ?? .controlAccentColor
        borderView.width = values.borderWidth
        borderView.needsDisplay = true
        focusedID = nil
        let active = values.focusBorder || values.dimBackground
        if active, timer == nil {
            // Window moves aren't observed, so poll while anything is shown.
            timer = Timer.scheduledTimer(withTimeInterval: 1.0 / 20, repeats: true) { [weak self] _ in
                MainActor.assumeIsolated { self?.refresh() }
            }
        } else if !active {
            timer?.invalidate()
            timer = nil
        }
        refresh()
    }

    func refresh() {
        let focused = Self.focusedWindow()
        let changed = focused?.id != focusedID
        focusedID = focused?.id
        let frame = focused.map { Self.cocoaRect($0.frame) }

        if values.focusBorder, let frame {
            let width = values.borderWidth
            border.setFrame(frame.insetBy(dx: -width, dy: -width), display: true)
            border.orderFrontRegardless()
        } else {
            border.orderOut(nil)
        }

        let screens = NSScreen.screens
        guard values.dimBackground, let focused, let frame else {
            dims.forEach { $0.orderOut(nil) }
            return
        }
        while dims.count < screens.count {
            let panel = Self.panel(level: .normal)
            panel.contentView = DimView()
            dims.append(panel)
        }
        for (index, panel) in dims.enumerated() {
            guard index < screens.count, let view = panel.contentView as? DimView else {
                panel.orderOut(nil)
                continue
            }
            let screen = screens[index].frame
            if panel.frame != screen { panel.setFrame(screen, display: false) }
            // The hole keeps the focused window clear even if macOS refuses to
            // order the dim below another app's window.
            let hole = frame.offsetBy(dx: -screen.minX, dy: -screen.minY)
            if view.hole != hole || view.amount != values.dimAmount {
                view.hole = hole
                view.amount = values.dimAmount
                view.needsDisplay = true
            }
            if changed || !panel.isVisible { panel.order(.below, relativeTo: Int(focused.id)) }
        }
    }

    /// The front window of the frontmost app, in Accessibility coordinates.
    private static func focusedWindow() -> (id: CGWindowID, frame: CGRect)? {
        guard let pid = NSWorkspace.shared.frontmostApplication?.processIdentifier,
              pid != ProcessInfo.processInfo.processIdentifier,
              let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)
                as? [[String: Any]] else { return nil }
        // Front to back; skip the app's invisible and tiny helper windows.
        for info in list {
            guard info[kCGWindowOwnerPID as String] as? pid_t == pid,
                  info[kCGWindowLayer as String] as? Int == 0,
                  (info[kCGWindowAlpha as String] as? Double ?? 1) > 0,
                  let id = info[kCGWindowNumber as String] as? CGWindowID,
                  let bounds = info[kCGWindowBounds as String] as? NSDictionary,
                  let frame = CGRect(dictionaryRepresentation: bounds as CFDictionary),
                  frame.width > 40, frame.height > 40 else { continue }
            return (id, frame)
        }
        return nil
    }

    private static func cocoaRect(_ rect: CGRect) -> CGRect {
        let primaryHeight = NSScreen.screens.first?.frame.height ?? 0
        return CGRect(x: rect.minX, y: primaryHeight - rect.maxY, width: rect.width, height: rect.height)
    }

    private static func panel(level: NSWindow.Level) -> NSPanel {
        let panel = NSPanel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered,
                            defer: true)
        panel.level = level
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.ignoresMouseEvents = true
        panel.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle, .fullScreenAuxiliary]
        return panel
    }
}

/// macOS window corners are about this round.
private let cornerRadius: CGFloat = 10

private final class BorderView: NSView {
    var color: NSColor = .controlAccentColor
    var width: CGFloat = 4

    override func draw(_ dirtyRect: NSRect) {
        let inset = width / 2
        let path = NSBezierPath(roundedRect: bounds.insetBy(dx: inset, dy: inset),
                                xRadius: cornerRadius + inset, yRadius: cornerRadius + inset)
        path.lineWidth = width
        color.setStroke()
        path.stroke()
    }
}

private final class DimView: NSView {
    var amount: Double = 0.3
    var hole: CGRect?

    override func draw(_ dirtyRect: NSRect) {
        let path = NSBezierPath(rect: bounds)
        if let hole {
            path.append(NSBezierPath(roundedRect: hole, xRadius: cornerRadius, yRadius: cornerRadius))
            path.windingRule = .evenOdd
        }
        NSColor.black.withAlphaComponent(amount).setFill()
        path.fill()
    }
}

extension NSColor {
    /// "#RRGGBB".
    convenience init?(hex: String) {
        guard hex.hasPrefix("#"), hex.count == 7, let rgb = Int(hex.dropFirst(), radix: 16) else { return nil }
        self.init(srgbRed: CGFloat(rgb >> 16 & 0xFF) / 255, green: CGFloat(rgb >> 8 & 0xFF) / 255,
                  blue: CGFloat(rgb & 0xFF) / 255, alpha: 1)
    }

    var hex: String {
        let c = usingColorSpace(.sRGB) ?? self
        let rgb = [c.redComponent, c.greenComponent, c.blueComponent].map { Int(($0 * 255).rounded()) }
        return String(format: "#%02X%02X%02X", rgb[0], rgb[1], rgb[2])
    }
}
