import AppKit

/// Reports where the user lets go of the mouse after dragging, in any app,
/// so a window dragged by its title bar can be dropped into the strip.
@MainActor
final class DragWatcher {
    private let onDrop: (CGPoint) -> Void
    private var monitor: Any?
    /// Whether the left button moved since it went down.
    private var dragged = false

    /// Whether the left button is down and has moved since it went down.
    /// Checks the button too, in case the release never reached us (the
    /// screen locked mid-drag, say), so syncs are not held back for good.
    var isDragging: Bool {
        dragged && NSEvent.pressedMouseButtons & 1 != 0
    }

    init(onDrop: @escaping (CGPoint) -> Void) {
        self.onDrop = onDrop
    }

    func start() {
        guard monitor == nil else { return }
        // Only other apps' windows are tiled, so the global monitor is enough.
        monitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDragged, .leftMouseUp]) { [weak self] event in
            MainActor.assumeIsolated { self?.handle(event) }
        }
    }

    private func handle(_ event: NSEvent) {
        if event.type == .leftMouseDragged {
            dragged = true
            return
        }
        guard dragged else { return }
        dragged = false
        // Where the button was released, not where the pointer is now.
        // CGEvent locations are in Accessibility coordinates already.
        guard let location = event.cgEvent?.location else { return }
        onDrop(location)
    }
}
