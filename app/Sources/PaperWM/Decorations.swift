import AppKit

/// Highlights the focused window: a colored border around it and a dim over
/// everything behind it. Both are click-through panels drawn with layers, so
/// moving them is left to the GPU. The engine says which window is focused;
/// the window server says where it is, which also covers windows PaperWM
/// doesn't tile and windows being dragged.
@MainActor
final class Decorations {
    private let border: NSPanel
    private let borderView = BorderView()
    /// One per screen, at normal window level just below the focused window.
    private var dims: [NSPanel] = []
    private var values = Settings.Values()
    /// Follows the focused window while a mouse button is down.
    private var timer: Timer?
    private var mouseMonitor: Any?
    /// The window being decorated and the frame it was last shown at.
    private var focused: (id: CGWindowID, frame: CGRect)?
    /// The focused window the engine last reported.
    private var engineFocus: CGWindowID?
    /// Measured corner radius of recently seen windows; nil while measuring
    /// or when it couldn't be measured.
    private var radii: [CGWindowID: CGFloat?] = [:]
    private static let ownPID = ProcessInfo.processInfo.processIdentifier

    private var isActive: Bool { values.focusBorder || values.dimBackground }

    init() {
        border = Self.panel(level: .floating)
        border.contentView = borderView
    }

    func apply(_ values: Settings.Values) {
        self.values = values
        borderView.color = NSColor(hex: values.borderColor) ?? .controlAccentColor
        borderView.width = values.borderWidth
        if isActive, mouseMonitor == nil {
            // Window moves aren't observed. The user moves windows by dragging
            // them, and slides and pans call `track` themselves, so polling
            // is only needed while a button is down.
            mouseMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .leftMouseUp]) {
                [weak self] event in
                MainActor.assumeIsolated { self?.followMouse(down: event.type == .leftMouseDown) }
            }
        } else if !isActive {
            if let mouseMonitor { NSEvent.removeMonitor(mouseMonitor) }
            mouseMonitor = nil
            followMouse(down: false)
        }
        focused = nil
        refresh(focus: engineFocus)
    }

    /// Finds the focused window again, after focus may have changed.
    /// `focus` is the engine's focused window, if it knows one.
    func refresh(focus: CGWindowID?) {
        engineFocus = focus
        guard isActive else {
            hide()
            return
        }
        let window = focus.flatMap(Self.window) ?? Self.frontWindow()
        show(window, raised: window?.id != focused?.id)
        // Without the polling timer, catch a window that settles a moment
        // after it was placed, e.g. one that refused its size.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { [weak self] in
            MainActor.assumeIsolated { self?.track() }
        }
    }

    private func followMouse(down: Bool) {
        timer?.invalidate()
        timer = nil
        guard down else {
            track()
            return
        }
        // Each tick looks up that one window.
        let timer = Timer(timeInterval: 1.0 / 20, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.track() }
        }
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
    }

    /// Follows the focused window when it moves, e.g. during a slide or a drag.
    func track() {
        guard isActive, let id = focused?.id else { return }
        let window = Self.window(id)
        if window?.frame != focused?.frame { show(window, raised: false) }
    }

    private func hide() {
        focused = nil
        border.orderOut(nil)
        dims.forEach { $0.orderOut(nil) }
    }

    private func show(_ window: (id: CGWindowID, frame: CGRect)?, raised: Bool) {
        guard let window else {
            hide()
            return
        }
        focused = window
        let frame = window.frame.flippedForScreens
        let radius = radius(of: window.id)

        if values.focusBorder {
            let width = values.borderWidth
            borderView.radius = radius
            border.setFrame(frame.insetBy(dx: -width, dy: -width), display: false)
            if raised || !border.isVisible { border.orderFrontRegardless() }
        } else {
            border.orderOut(nil)
        }

        let screens = NSScreen.screens
        guard values.dimBackground else {
            dims.forEach { $0.orderOut(nil) }
            return
        }
        let shared = { (screen: NSScreen) -> CGFloat in
            let overlap = frame.intersection(screen.frame)
            return overlap.isNull ? 0 : overlap.width * overlap.height
        }
        let owner = screens.max { shared($0) < shared($1) }
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
            // order the dim below another app's window. With separate Spaces,
            // the window shows only on the screen holding most of it.
            let shown = !NSScreen.screensHaveSeparateSpaces || screens[index] == owner
            view.update(hole: shown ? frame.offsetBy(dx: -screen.minX, dy: -screen.minY) : .zero, radius: radius,
                        amount: values.dimAmount)
            if raised || !panel.isVisible { panel.order(.below, relativeTo: Int(window.id)) }
        }
    }

    /// The window's own corner radius once measured, the setting until then.
    private func radius(of id: CGWindowID) -> CGFloat {
        if let measured = radii[id] {
            return measured ?? CGFloat(values.cornerRadius)
        }
        // Window ids aren't reused for a long time; forgetting old ones now
        // and then keeps this small.
        if radii.count > 256 { radii.removeAll() }
        radii[id] = .some(nil)
        Task { [weak self] in
            let measured = await WindowCorners.radius(of: id)
            guard let self else { return }
            self.radii[id] = .some(measured)
            if measured != nil, self.focused?.id == id { self.show(self.focused, raised: false) }
        }
        return CGFloat(values.cornerRadius)
    }

    /// Where window `id` is, in Accessibility coordinates, if it is another
    /// app's window on screen.
    private static func window(_ id: CGWindowID) -> (id: CGWindowID, frame: CGRect)? {
        guard let list = CGWindowListCopyWindowInfo([.optionIncludingWindow], id) as? [[String: Any]],
              let info = list.first,
              info[kCGWindowOwnerPID as String] as? pid_t != ownPID,
              info[kCGWindowIsOnscreen as String] as? Bool ?? false,
              let bounds = info[kCGWindowBounds as String] as? NSDictionary,
              let frame = CGRect(dictionaryRepresentation: bounds as CFDictionary) else { return nil }
        return (id, frame)
    }

    /// The front window of the frontmost app, for when the engine doesn't
    /// know the focused window.
    private static func frontWindow() -> (id: CGWindowID, frame: CGRect)? {
        guard let pid = NSWorkspace.shared.frontmostApplication?.processIdentifier, pid != ownPID,
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

/// A rounded ring drawn by a layer, so resizing it redraws nothing.
private final class BorderView: NSView {
    private let ring = CALayer()

    var color: NSColor = .controlAccentColor {
        didSet { withoutAnimation { ring.borderColor = color.cgColor } }
    }
    var width: CGFloat = 4 {
        didSet { withoutAnimation { ring.borderWidth = width; ring.cornerRadius = radius + width } }
    }
    /// The window's corner radius; the ring's outer edge is rounder by `width`.
    var radius: CGFloat = 10 {
        didSet { if radius != oldValue { withoutAnimation { ring.cornerRadius = radius + width } } }
    }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        ring.borderColor = color.cgColor
        ring.borderWidth = width
        ring.cornerRadius = radius + width
        layer?.addSublayer(ring)
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    override func layout() {
        super.layout()
        withoutAnimation { ring.frame = bounds }
    }
}

/// A translucent black cover with a rounded hole, drawn by a shape layer.
private final class DimView: NSView {
    private let shape = CAShapeLayer()
    private var hole: CGRect?
    private var radius: CGFloat = 0

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        shape.fillRule = .evenOdd
        layer?.addSublayer(shape)
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    func update(hole: CGRect, radius: CGFloat, amount: Double) {
        let color = NSColor.black.withAlphaComponent(amount).cgColor
        guard hole != self.hole || radius != self.radius || shape.fillColor != color || shape.frame != bounds else {
            return
        }
        self.hole = hole
        self.radius = radius
        withoutAnimation {
            shape.frame = bounds
            shape.fillColor = color
            let path = CGMutablePath()
            path.addRect(bounds)
            let corner = min(radius, hole.width / 2, hole.height / 2)
            path.addRoundedRect(in: hole, cornerWidth: max(corner, 0), cornerHeight: max(corner, 0))
            shape.path = path
        }
    }
}

/// Applies layer changes at once instead of with Core Animation's implicit
/// quarter-second animation.
private func withoutAnimation(_ changes: () -> Void) {
    CATransaction.begin()
    CATransaction.setDisableActions(true)
    changes()
    CATransaction.commit()
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
