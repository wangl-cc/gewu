import AppKit
import GewuKit
import SwiftUI

/// Everything the native table presents, as one value. SwiftUI rebuilds it
/// when any observed input changes; the controller applies only what differs.
struct TablePresentation: Equatable {
    struct Style: Equatable {
        var fontSize: CGFloat
        var rowHeight: CGFloat
        var showsRowNumbers: Bool
        var rowNumberBase: Int
    }

    struct FindDecoration: Equatable {
        var query: String?
        var currentMatch: CellMatch?
        var highlights: HighlightBlocks
    }

    /// The rows to show. The table keeps showing its current rows until the
    /// viewport of new ones is read, then switches in one step.
    var rows: DisplayRows
    var columns: [ColumnID]
    var selectedColumn: ColumnID?
    /// `nil` while the Inspector is hidden.
    var inspectedColumn: ColumnID?
    var find: FindDecoration
    var style: Style
    var reveal: RevealRequest?
    var filterEditor: ColumnID?
}

extension TablePresentation {
    /// What `model` asks the table to show. Reads only the state the table
    /// presents, so unrelated model changes do not update the table.
    @MainActor
    init(model: TableModel, inspectorVisible: Bool, style: Style) {
        self.init(
            rows: model.displayed,
            columns: model.visibleColumns,
            selectedColumn: model.selectedColumn,
            inspectedColumn: inspectorVisible ? model.inspectedColumn : nil,
            find: FindDecoration(
                query: model.find.activeQuery, currentMatch: model.find.currentMatch,
                highlights: model.find.highlights),
            style: style,
            reveal: model.reveal,
            filterEditor: model.filterEditor)
    }
}

/// The table area of a window: a native `NSTableView` driven by ``TableModel``.
struct DataTable: View {
    let model: TableModel
    let inspectorVisible: Bool
    /// Show a column's analysis in the window's Inspector.
    let analyze: (ColumnID) -> Void
    @AppStorage(Preferences.tableFontSize) private var fontSize: Double
    @AppStorage(Preferences.tableRowHeight) private var rowHeight: Double
    @AppStorage(Preferences.showsRowNumbers) private var showsRowNumbers: Bool
    @AppStorage(Preferences.rowNumberBase) private var rowNumberBase: Int

    var body: some View {
        TableRepresentable(
            model: model, analyze: analyze,
            presentation: TablePresentation(
                model: model, inspectorVisible: inspectorVisible,
                style: .init(
                    fontSize: fontSize.clamped(to: Preferences.fontSizes),
                    rowHeight: rowHeight.clamped(to: Preferences.rowHeights),
                    showsRowNumbers: showsRowNumbers, rowNumberBase: rowNumberBase)))
    }
}

private struct TableRepresentable: NSViewControllerRepresentable {
    let model: TableModel
    let analyze: (ColumnID) -> Void
    let presentation: TablePresentation

    func makeNSViewController(context: Context) -> TableViewController {
        TableViewController(model: model, presentation: presentation, analyze: analyze)
    }

    func updateNSViewController(_ controller: TableViewController, context: Context) {
        controller.present(presentation)
    }

    static func dismantleNSViewController(_ controller: TableViewController, coordinator: ()) {
        controller.stop()
    }
}

extension Comparable {
    fileprivate func clamped(to range: ClosedRange<Self>) -> Self {
        min(max(self, range.lowerBound), range.upperBound)
    }
}
