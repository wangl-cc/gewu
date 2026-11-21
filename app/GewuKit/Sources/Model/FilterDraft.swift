/// A value of an exact filter. Compares by UTF-8 bytes, as the engine does,
/// unlike `String`, whose equality merges canonically equivalent spellings.
public struct ExactValue: Hashable, Sendable {
    public let text: String

    public init(_ text: String) {
        self.text = text
    }

    public static func == (lhs: Self, rhs: Self) -> Bool { lhs.text.utf8.elementsEqual(rhs.text.utf8) }

    public func hash(into hasher: inout Hasher) { hashBytes(text, into: &hasher) }
}

extension ValueChoice {
    public var exactValue: ExactValue { ExactValue(value) }
}

/// The editors a column's filter can be written with.
public enum FilterMode: CaseIterable, Hashable, Sendable {
    /// Choose from the column's values.
    case exact
    /// Inclusive bounds.
    case range
    /// Match text.
    case text

    /// The modes `capabilities` and `choices` allow. Value selection needs a
    /// complete list of values.
    public static func available(capabilities: ColumnCapabilities, choices: FilterChoices?) -> [FilterMode] {
        allCases.filter {
            switch $0 {
            case .range: capabilities.range
            case .text: capabilities.textMatch
            case .exact: capabilities.exact && choices?.values != nil
            }
        }
    }

    /// The mode an editor opens in: the current filter's own mode when still
    /// offered, else value selection when the column has a complete list of
    /// values, else Range, then Text. `nil` while `choices` are needed to decide.
    public static func initial(
        capabilities: ColumnCapabilities, current: Filter?, choices: FilterChoices? = nil
    ) -> FilterMode? {
        let selectable: Bool? = capabilities.exact ? choices.map { $0.values != nil } : false
        if let current {
            switch current.condition {
            case .range: if capabilities.range { return .range }
            case .text: if capabilities.textMatch { return .text }
            case .exact:
                guard let selectable else { return nil }
                if selectable { return .exact }
            case nil: break
            }
        }
        guard let selectable else { return nil }
        if selectable { return .exact }
        return capabilities.range ? .range : (capabilities.textMatch ? .text : .exact)
    }
}

/// How an edited filter treats missing values.
public enum NullMode: Hashable, Sendable {
    case include, exclude, only

    public init(_ filter: Filter?) {
        switch filter {
        case .onlyNulls: self = .only
        case .notNull: self = .exclude
        case .condition(_, let includeNulls): self = includeNulls ? .include : .exclude
        case nil: self = .include
        }
    }

    /// The filter for `condition` under this mode; `nil` when it lets every
    /// row pass. Excluding no values is no condition.
    public func applying(to condition: FilterCondition?) -> Filter? {
        var condition = condition
        if case .exact(let values, true) = condition, values.isEmpty { condition = nil }
        switch (self, condition) {
        case (.only, _): return .onlyNulls
        case (.exclude, nil): return .notNull
        case (.include, nil): return nil
        case (_, let condition?): return .condition(condition, includeNulls: self == .include)
        }
    }
}

/// Values chosen for an exact filter: the listed values, or with `exclude`,
/// every value except them.
public struct ExactSelection: Sendable {
    public private(set) var exclude = true
    public private(set) var values: Set<ExactValue> = []

    public init(_ filter: Filter?) {
        if case .exact(let values, let exclude) = filter?.condition {
            self.values = Set(values.map(ExactValue.init))
            self.exclude = exclude
        }
    }

    public func contains(_ value: ExactValue) -> Bool { values.contains(value) != exclude }

    public mutating func set(_ value: ExactValue, selected: Bool) {
        if selected != exclude { values.insert(value) } else { values.remove(value) }
    }

    public mutating func setAll(_ selected: Bool) {
        exclude = selected
        values.removeAll()
    }

    /// Values in byte order, so equal selections give equal conditions.
    public var condition: FilterCondition {
        .exact(values: values.map(\.text).sorted { $0.utf8.lexicographicallyPrecedes($1.utf8) }, exclude: exclude)
    }
}

public struct TextMatchDraft: Sendable {
    public var text = ""
    public var mode = TextMatch.contains
    public var caseSensitive = false

    public init(_ filter: Filter?) {
        if case .text(let text, let mode, let caseSensitive) = filter?.condition {
            self.text = text
            self.mode = mode
            self.caseSensitive = caseSensitive
        }
    }

    /// `nil` without text.
    public var condition: FilterCondition? {
        text.isEmpty ? nil : .text(text: text, mode: mode, caseSensitive: caseSensitive)
    }
}

/// Range bounds as the user edits them: typed text, or bin edges chosen on a
/// histogram, whose exact values are kept apart from their labels.
public struct RangeDraft: Sendable {
    /// The text shown for each bound.
    public private(set) var minimum = ""
    public private(set) var maximum = ""
    /// The bin edges chosen on a histogram, when a bound was not typed.
    public private(set) var lowerBound: RangeBound?
    public private(set) var upperBound: RangeBound?
    /// Whether each bound's text was typed rather than chosen.
    public private(set) var lowerTyped = false
    public private(set) var upperTyped = false

    public init(_ filter: Filter?) {
        if case .range(let min, let max) = filter?.condition {
            minimum = min ?? ""
            maximum = max ?? ""
            lowerTyped = true
            upperTyped = true
        }
    }

    public var isTyped: Bool { lowerTyped || upperTyped }

    public mutating func typeMinimum(_ text: String) {
        minimum = text
        lowerTyped = true
    }

    public mutating func typeMaximum(_ text: String) {
        maximum = text
        upperTyped = true
    }

    /// Choose bins `bins` of `histogram`, using their exact edges.
    public mutating func select(_ bins: Range<Int>, of histogram: RangeHistogram) {
        lowerBound = histogram.ranges[bins.lowerBound].min
        upperBound = histogram.ranges[bins.upperBound - 1].max
        minimum = lowerBound?.label ?? ""
        maximum = upperBound?.label ?? ""
        lowerTyped = false
        upperTyped = false
    }

    /// Show typed bounds that equal one of `histogram`'s exact edges by that
    /// edge's label, such as when an applied range filter is reopened.
    public mutating func restoreLabels(from histogram: RangeHistogram?) {
        guard let histogram else { return }
        if lowerTyped, let bound = histogram.ranges.compactMap(\.min).first(where: { $0.value.utf8.elementsEqual(minimum.utf8) }) {
            lowerBound = bound
            minimum = bound.label
            lowerTyped = false
        }
        if upperTyped, let bound = histogram.ranges.compactMap(\.max).first(where: { $0.value.utf8.elementsEqual(maximum.utf8) }) {
            upperBound = bound
            maximum = bound.label
            upperTyped = false
        }
    }

    /// `nil` without bounds.
    public var condition: FilterCondition? {
        let min = lowerTyped ? minimum.nilIfBlank : lowerBound?.value
        let max = upperTyped ? maximum.nilIfBlank : upperBound?.value
        return min == nil && max == nil ? nil : .range(min: min, max: max)
    }
}

extension String {
    fileprivate var nilIfBlank: String? {
        let text = trimmingCharacters(in: .whitespacesAndNewlines)
        return text.isEmpty ? nil : text
    }
}

/// One column's filter while its editor is open. Each mode keeps its own
/// draft, so switching modes loses nothing; the null mode is shared.
public struct FilterDraft: Sendable {
    public var exact: ExactSelection
    public var text: TextMatchDraft
    public var range: RangeDraft
    public var nullMode: NullMode

    public init(_ filter: Filter?) {
        exact = ExactSelection(filter)
        text = TextMatchDraft(filter)
        range = RangeDraft(filter)
        nullMode = NullMode(filter)
    }

    /// Value selection lists missing values as one more value.
    public var includesMissing: Bool {
        get { nullMode != .exclude }
        set { nullMode = newValue ? .include : .exclude }
    }

    /// Choosing a value leaves the null-only mode.
    public mutating func setValue(_ value: ExactValue, selected: Bool) {
        enterValues()
        exact.set(value, selected: selected)
    }

    public mutating func selectAllValues(_ selected: Bool) {
        exact.setAll(selected)
        includesMissing = selected
    }

    /// Value selection shows missing values as a value, so it cannot show
    /// the null-only mode; entering it starts from nothing selected.
    public mutating func enterValues() {
        guard nullMode == .only else { return }
        exact.setAll(false)
        nullMode = .include
    }

    public mutating func reset() { self = Self(nil) }

    /// The filter `mode`'s draft describes. Without `hasContent`, for columns
    /// with no content filters, only the null mode applies.
    public func filter(mode: FilterMode, hasContent: Bool = true) -> Filter? {
        let condition: FilterCondition? =
            switch mode {
            case .exact: exact.condition
            case .text: text.condition
            case .range: range.condition
            }
        return nullMode.applying(to: hasContent ? condition : nil)
    }
}
