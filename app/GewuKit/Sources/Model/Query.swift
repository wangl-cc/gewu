internal import GewuBindings

/// How a text filter compares values.
public enum TextMatch: Hashable, Sendable, Codable {
    case contains, equals
}

/// The condition one column's values must satisfy. Literals are sent to Core
/// unchanged; Core parses them for the column's type.
///
/// Conditions compare their literals by UTF-8 bytes, as Core does, so two
/// conditions are equal exactly when Core treats them alike. `String`
/// equality would merge canonically equivalent spellings, such as a
/// precomposed and a decomposed "é", which Core keeps apart.
public enum FilterCondition: Hashable, Sendable, Codable {
    /// Inclusive bounds. Use ``RangeBound/value`` for exact endpoints, never display labels.
    case range(min: String?, max: String?)
    case text(text: String, mode: TextMatch, caseSensitive: Bool)
    /// Exact values.
    case exact(values: [String], exclude: Bool)

    public static func == (lhs: Self, rhs: Self) -> Bool {
        switch (lhs, rhs) {
        case (.range(let min, let max), .range(let otherMin, let otherMax)):
            sameBytes(min, otherMin) && sameBytes(max, otherMax)
        case (.text(let text, let mode, let caseSensitive), .text(let otherText, let otherMode, let otherCase)):
            text.sameBytes(as: otherText) && mode == otherMode && caseSensitive == otherCase
        case (.exact(let values, let exclude), .exact(let otherValues, let otherExclude)):
            exclude == otherExclude && values.count == otherValues.count
                && zip(values, otherValues).allSatisfy { $0.sameBytes(as: $1) }
        default: false
        }
    }

    public func hash(into hasher: inout Hasher) {
        switch self {
        case .range(let min, let max):
            hasher.combine(0)
            hashBytes(min, into: &hasher)
            hashBytes(max, into: &hasher)
        case .text(let text, let mode, let caseSensitive):
            hasher.combine(1)
            hashBytes(text, into: &hasher)
            hasher.combine(mode)
            hasher.combine(caseSensitive)
        case .exact(let values, let exclude):
            hasher.combine(2)
            hasher.combine(exclude)
            for value in values { hashBytes(value, into: &hasher) }
        }
    }
}

private func sameBytes(_ lhs: String?, _ rhs: String?) -> Bool {
    switch (lhs, rhs) {
    case (nil, nil): true
    case (let lhs?, let rhs?): lhs.sameBytes(as: rhs)
    default: false
    }
}

extension String {
    /// Whether the two strings have the same UTF-8 bytes, as Core compares
    /// literals, unlike `==`, which merges canonically equivalent spellings.
    func sameBytes(as other: String) -> Bool { utf8.elementsEqual(other.utf8) }
}

/// Hash `text` by its UTF-8 bytes, consistently with ``Swift/String/sameBytes(as:)``.
func hashBytes(_ text: String?, into hasher: inout Hasher) {
    guard let text else {
        hasher.combine(-1)
        return
    }
    hasher.combine(text.utf8.count)
    for byte in text.utf8 { hasher.combine(byte) }
}

/// Which rows of one column pass.
public enum Filter: Hashable, Sendable, Codable {
    /// Rows whose value is missing.
    case onlyNulls
    /// Rows whose value is not missing.
    case notNull
    /// Rows whose value satisfies the condition, and rows with missing values
    /// when `includeNulls`.
    case condition(FilterCondition, includeNulls: Bool)

    /// The condition on present values; `nil` for the null-only modes.
    public var condition: FilterCondition? {
        if case .condition(let condition, _) = self { condition } else { nil }
    }

    /// Whether rows with missing values pass.
    public var passesNulls: Bool {
        switch self {
        case .onlyNulls: true
        case .notNull: false
        case .condition(_, let includeNulls): includeNulls
        }
    }
}

public struct ColumnFilter: Hashable, Sendable, Codable {
    public var column: ColumnID
    public var filter: Filter

    public init(column: ColumnID, filter: Filter) {
        self.column = column
        self.filter = filter
    }
}

public struct ColumnSort: Hashable, Sendable, Codable {
    public var column: ColumnID
    public var descending: Bool

    public init(column: ColumnID, descending: Bool) {
        self.column = column
        self.descending = descending
    }
}

/// The rows a table shows: which rows pass and in what order. Column
/// visibility is presentation only and is not part of a query.
public struct TableQuery: Hashable, Sendable {
    /// At most one filter per column, kept in column order whatever order
    /// they were set in. Their order does not affect the rows, so keeping one
    /// order makes queries with the same filters compare equal.
    public private(set) var filters: [ColumnFilter]
    /// Sort keys by priority, at most one per column.
    public var sorts: [ColumnSort]

    public init(filters: [ColumnFilter] = [], sorts: [ColumnSort] = []) {
        precondition(Set(filters.map(\.column)).count == filters.count, "at most one filter per column")
        self.filters = filters.sorted { $0.column.raw < $1.column.raw }
        self.sorts = sorts
    }

    public func filter(for column: ColumnID) -> Filter? {
        filters.first { $0.column == column }?.filter
    }

    public func sort(for column: ColumnID) -> ColumnSort? {
        sorts.first { $0.column == column }
    }

    /// Set or, with `nil`, remove `column`'s filter.
    public mutating func setFilter(_ filter: Filter?, for column: ColumnID) {
        filters.removeAll { $0.column == column }
        guard let filter else { return }
        let index = filters.firstIndex { $0.column.raw > column.raw } ?? filters.endIndex
        filters.insert(ColumnFilter(column: column, filter: filter), at: index)
    }

    public mutating func removeFilters(where shouldRemove: (ColumnFilter) -> Bool) {
        filters.removeAll(where: shouldRemove)
    }
}

// MARK: - Conversion to the bindings

extension TextMatch {
    var ffi: GewuBindings.TextMatch {
        switch self {
        case .contains: .contains
        case .equals: .equals
        }
    }
}

extension FilterCondition {
    var ffi: GewuBindings.FilterCondition {
        switch self {
        case .range(let min, let max): .range(min: min, max: max)
        case .text(let text, let mode, let caseSensitive):
            .text(text: text, mode: mode.ffi, caseSensitive: caseSensitive)
        case .exact(let values, let exclude): .exact(values: values, exclude: exclude)
        }
    }
}

extension Filter {
    var ffi: GewuBindings.Filter {
        switch self {
        case .onlyNulls: .onlyNulls
        case .notNull: .notNull
        case .condition(let condition, let includeNulls):
            .condition(condition: condition.ffi, includeNulls: includeNulls)
        }
    }
}

extension ColumnFilter {
    var ffi: GewuBindings.ColumnFilter { GewuBindings.ColumnFilter(column: column.ffi, filter: filter.ffi) }
}

extension ColumnSort {
    var ffi: GewuBindings.ColumnSort { GewuBindings.ColumnSort(column: column.ffi, descending: descending) }
}
