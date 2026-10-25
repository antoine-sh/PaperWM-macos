import AppKit

/// Reports where the user lets go of the mouse after dragging, in any app,
/// so a window dragged by its title bar can be dropped into the strip.
@MainActor
final class DragWatcher {
    private let onDrop: (CGPoint) -> Void
    private var monitor: Any?
    /// Whether the left button is down and has moved since it went down.
    private(set) var isDragging = false

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
            isDragging = true
            return
        }
        guard isDragging else { return }
        isDragging = false
        // CGEvent locations are in Accessibility coordinates already.
        guard let location = CGEvent(source: nil)?.location else { return }
        onDrop(location)
    }
}
