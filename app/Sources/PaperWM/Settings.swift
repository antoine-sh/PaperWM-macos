import Foundation

enum MinimapStyle: String, Codable {
    /// The strip with each window as a box, in the middle of the screen.
    case strip
    /// A row of app icons at the bottom of the screen.
    case toolbar
}

/// The user's preferences, stored as JSON in `UserDefaults`.
@MainActor
final class Settings: ObservableObject {
    struct Values: Codable, Equatable {
        var gap: Double = 8
        /// Fractions of the screen width that Cycle Column Width steps through.
        var widthPresets: [Double] = [1.0 / 3.0, 0.5, 2.0 / 3.0]
        var workspaceNames: [String] = (1...9).map { String($0) }
        /// Seconds the strip takes to scroll; 0 jumps.
        var animationDuration: Double = 0.25
        /// Shortcut overrides by `HotKey.id`; missing ids use the default.
        var shortcuts: [String: KeyCombo] = [:]
        /// Draw a colored border around the focused window.
        var focusBorder = false
        /// "#RRGGBB".
        var borderColor = "#3D8BFF"
        var borderWidth: Double = 4
        /// Window corner radius used until a window's own is measured, or
        /// when it can't be (no Screen Recording permission).
        var cornerRadius: Double = ProcessInfo.processInfo.operatingSystemVersion.majorVersion >= 26 ? 16 : 10
        /// Dim every window except the focused one.
        var dimBackground = false
        /// Opacity of the dim, 0–1.
        var dimAmount = 0.3
        var minimapStyle: MinimapStyle = .toolbar

        init() {}

        // Missing keys keep their defaults, so older settings still load.
        init(from decoder: Decoder) throws {
            let defaults = Values()
            let c = try decoder.container(keyedBy: CodingKeys.self)
            gap = try c.decodeIfPresent(Double.self, forKey: .gap) ?? defaults.gap
            widthPresets = try c.decodeIfPresent([Double].self, forKey: .widthPresets) ?? defaults.widthPresets
            workspaceNames = try c.decodeIfPresent([String].self, forKey: .workspaceNames) ?? defaults.workspaceNames
            animationDuration = try c.decodeIfPresent(Double.self, forKey: .animationDuration)
                ?? defaults.animationDuration
            shortcuts = try c.decodeIfPresent([String: KeyCombo].self, forKey: .shortcuts) ?? defaults.shortcuts
            focusBorder = try c.decodeIfPresent(Bool.self, forKey: .focusBorder) ?? defaults.focusBorder
            borderColor = try c.decodeIfPresent(String.self, forKey: .borderColor) ?? defaults.borderColor
            borderWidth = try c.decodeIfPresent(Double.self, forKey: .borderWidth) ?? defaults.borderWidth
            cornerRadius = try c.decodeIfPresent(Double.self, forKey: .cornerRadius) ?? defaults.cornerRadius
            dimBackground = try c.decodeIfPresent(Bool.self, forKey: .dimBackground) ?? defaults.dimBackground
            dimAmount = try c.decodeIfPresent(Double.self, forKey: .dimAmount) ?? defaults.dimAmount
            minimapStyle = try c.decodeIfPresent(MinimapStyle.self, forKey: .minimapStyle) ?? defaults.minimapStyle
            workspaceNames += defaults.workspaceNames.dropFirst(workspaceNames.count)
        }
    }

    @Published var values: Values {
        didSet {
            guard values != oldValue else { return }
            save()
            onChange?()
        }
    }

    /// Called after every change.
    var onChange: (() -> Void)?

    private static let key = "settings"

    init() {
        let data = UserDefaults.standard.data(forKey: Settings.key)
        values = data.flatMap { try? JSONDecoder().decode(Values.self, from: $0) } ?? Values()
    }

    /// The default shortcuts with the user's overrides applied.
    var hotKeys: [HotKey] {
        HotKey.defaults.map { hotKey in
            var hotKey = hotKey
            hotKey.combo = values.shortcuts[hotKey.id] ?? hotKey.combo
            return hotKey
        }
    }

    /// The part of the settings the engine uses, as `engine::Config` JSON.
    var engineConfigJSON: String {
        struct Config: Encodable {
            let gap: Double
            let widthPresets: [Double]
            let workspaceNames: [String]
            let animationDuration: Double
        }
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        let config = Config(gap: values.gap, widthPresets: values.widthPresets, workspaceNames: values.workspaceNames,
                            animationDuration: values.animationDuration)
        return (try? encoder.encode(config)).map { String(decoding: $0, as: UTF8.self) } ?? "{}"
    }

    private func save() {
        if let data = try? JSONEncoder().encode(values) {
            UserDefaults.standard.set(data, forKey: Settings.key)
        }
    }
}
