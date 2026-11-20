internal import GewuBindings

/// How a text filter compares values.
public enum TextMatch: Hashable, Sendable, Codable {
    case contains, equals
}

/// The condition one column's values must satisfy. Literals are sent to Core
/// unchanged; Core parses them for the column's type.
public enum FilterCondition: Hashable, Sendable, Codable {
    /// Inclusive bounds. Use ``RangeBound/value`` for exact endpoints, never display labels.
    case range(min: String?, max: String?)
    case text(text: String, mode: TextMatch, caseSensitive: Bool)
    /// Exact values. Compare them by UTF-8 bytes, as Core does.
    case exact(values: [String], exclude: Bool)
}

public struct Filter: Hashable, Sendable, Codable {
    public var condition: FilterCondition
    /// Whether missing values pass, independently of `condition`.
    public var includeNulls: Bool

    public init(condition: FilterCondition, includeNulls: Bool) {
        self.condition = condition
        self.includeNulls = includeNulls
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
public struct TableQuery: Hashable, Sendable, Codable {
    /// At most one filter per column. Order does not affect the result.
    public var filters: [ColumnFilter]
    /// Sort keys by priority, at most one per column.
    public var sorts: [ColumnSort]

    public init(filters: [ColumnFilter] = [], sorts: [ColumnSort] = []) {
        self.filters = filters
        self.sorts = sorts
    }

    public func filter(for column: ColumnID) -> Filter? {
        filters.first { $0.column == column }?.filter
    }

    public func sort(for column: ColumnID) -> ColumnSort? {
        sorts.first { $0.column == column }
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

extension Filter {
    var ffi: GewuBindings.Filter {
        let condition: GewuBindings.FilterCondition =
            switch condition {
            case .range(let min, let max): .range(min: min, max: max)
            case .text(let text, let mode, let caseSensitive):
                .text(text: text, mode: mode.ffi, caseSensitive: caseSensitive)
            case .exact(let values, let exclude): .exact(values: values, exclude: exclude)
            }
        return GewuBindings.Filter(condition: condition, includeNulls: includeNulls)
    }
}

extension ColumnFilter {
    var ffi: GewuBindings.ColumnFilter { GewuBindings.ColumnFilter(column: column.ffi, filter: filter.ffi) }
}

extension ColumnSort {
    var ffi: GewuBindings.ColumnSort { GewuBindings.ColumnSort(column: column.ffi, descending: descending) }
}
