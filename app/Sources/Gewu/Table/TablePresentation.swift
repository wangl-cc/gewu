import Combine

/// The table observes only inputs that affect its rows, columns or header decoration.
/// Window panels and pending query state do not invalidate this presentation.
@MainActor
final class TablePresentation: ObservableObject {
    struct Input: Equatable {
        let rows: GewuView
        let spec: QuerySpec
        let selectedColumn: Int?
        let inspectedColumn: Int?

        static func == (lhs: Self, rhs: Self) -> Bool {
            lhs.rows === rhs.rows && lhs.spec == rhs.spec
                && lhs.selectedColumn == rhs.selectedColumn && lhs.inspectedColumn == rhs.inspectedColumn
        }
    }

    @Published private(set) var input: Input

    init(_ input: Input) { self.input = input }

    func apply(_ next: Input) {
        if input != next { input = next }
    }
}
