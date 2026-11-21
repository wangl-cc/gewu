import AppKit
import GewuKit
import SwiftUI

/// Header menus, filter editors and column widths.
extension TableViewController: NSPopoverDelegate {
    // MARK: Header menu

    func columnMenu(at index: Int) -> NSMenu? {
        guard let column = columnID(at: index) else { return nil }
        let sort = model.draft.sort(for: column)
        let menu = NSMenu()
        menu.addItem(menuItem("Analyze Column", #selector(analyzeMenuColumn(_:)), column))
        menu.addItem(menuItem("Filter Column…", #selector(filterMenuColumn(_:)), column))
        if model.filteredColumns.contains(column) {
            menu.addItem(menuItem("Clear Column Filter", #selector(clearMenuColumnFilter(_:)), column))
        }
        menu.addItem(.separator())
        let ascending = menuItem("Sort Ascending", #selector(sortMenuAscending(_:)), column)
        ascending.state = sort?.descending == false ? .on : .off
        menu.addItem(ascending)
        let descending = menuItem("Sort Descending", #selector(sortMenuDescending(_:)), column)
        descending.state = sort?.descending == true ? .on : .off
        menu.addItem(descending)
        if sort != nil { menu.addItem(menuItem("Clear Column Sort", #selector(clearMenuColumnSort(_:)), column)) }
        if !model.draft.sorts.isEmpty {
            menu.addItem(menuItem("Clear All Sorting", #selector(clearMenuAllSorting(_:)), column))
        }
        menu.addItem(.separator())
        menu.addItem(menuItem("Fit Column Width", #selector(fitMenuColumn(_:)), column))
        menu.addItem(menuItem("Fit All Columns", #selector(fitMenuAll(_:)), column))
        menu.addItem(.separator())
        menu.addItem(menuItem("Customize Columns…", #selector(customizeMenuColumns(_:)), column))
        menu.addItem(menuItem("Hide Column", #selector(hideMenuColumn(_:)), column))
        return menu
    }

    @objc func analyzeMenuColumn(_ item: NSMenuItem) {
        if let column = item.representedObject as? ColumnID { analyze(column) }
    }

    @objc func filterMenuColumn(_ item: NSMenuItem) {
        if let column = item.representedObject as? ColumnID { model.editFilter(column) }
    }

    @objc func clearMenuColumnFilter(_ item: NSMenuItem) {
        if let column = item.representedObject as? ColumnID { model.setFilter(nil, column: column) }
    }

    @objc private func sortMenuAscending(_ item: NSMenuItem) {
        if let column = item.representedObject as? ColumnID {
            model.setSortDirection(column: column, descending: false)
        }
    }

    @objc private func sortMenuDescending(_ item: NSMenuItem) {
        if let column = item.representedObject as? ColumnID { model.setSortDirection(column: column, descending: true) }
    }

    @objc private func clearMenuColumnSort(_ item: NSMenuItem) {
        if let column = item.representedObject as? ColumnID { model.removeSort(column: column) }
    }

    @objc private func clearMenuAllSorting(_ item: NSMenuItem) { model.clearSorts() }

    @objc private func fitMenuColumn(_ item: NSMenuItem) {
        if let column = item.representedObject as? ColumnID { fitColumns([column], automatic: false) }
    }

    @objc private func fitMenuAll(_ item: NSMenuItem) { fitColumns(displayedColumns, automatic: false) }

    @objc private func customizeMenuColumns(_ item: NSMenuItem) { model.showsColumns = true }

    @objc private func hideMenuColumn(_ item: NSMenuItem) {
        if let column = item.representedObject as? ColumnID { hideColumn(column, in: model) }
    }

    // MARK: Filter editor

    func editFilter(at index: Int) {
        if let column = columnID(at: index) { model.editFilter(column) }
    }

    /// Show the filter editor the model asks for, anchored at its header.
    func presentFilterEditor(_ column: ColumnID?) {
        guard filterPopover?.column != column else { return }
        closeFilterEditor()
        guard let column, let info = model.column(column), let header = table.headerView else { return }
        let index = table.column(withIdentifier: identifier(for: column))
        guard index >= 0 else { return }
        table.scrollColumnToVisible(index)
        let content = ColumnFilterView(model: model, info: info, current: model.draft.filter(for: column)) {
            [weak self] filter in
            guard let self else { return }
            model.filterEditor = nil
            model.setFilter(filter, column: column)
        }
        let popover = NSPopover()
        popover.behavior = .transient
        popover.delegate = self
        popover.contentViewController = NSHostingController(rootView: content)
        popover.show(relativeTo: header.headerRect(ofColumn: index), of: header, preferredEdge: .maxY)
        filterPopover = (column, popover)
    }

    func closeFilterEditor() {
        let popover = filterPopover?.popover
        filterPopover = nil
        popover?.close()
    }

    func popoverDidClose(_ notification: Notification) {
        guard let popover = notification.object as? NSPopover, popover === filterPopover?.popover,
            let column = filterPopover?.column
        else { return }
        filterPopover = nil
        if model.filterEditor == column { model.filterEditor = nil }
    }

    // MARK: Column widths

    func stopFitting() {
        fitTask?.cancel()
        automaticFitTask?.cancel()
    }

    /// Fit visible columns that have not been fitted or resized yet. The table
    /// stays transparent until the first visible columns are fitted.
    func fitVisibleColumns() {
        guard !fittingAutomatically, view.window != nil, presentation.rows === shownRows else { return }
        let columns = visibleColumns(in: table.visibleRect).filter { widths.needsAutomatic($0) }
        guard !columns.isEmpty else {
            if firstWidthsPending {
                table.alphaValue = 1
                firstWidthsPending = false
            }
            return
        }
        fitColumns(columns, automatic: true)
    }

    func fitColumns(_ columns: [ColumnID], automatic: Bool) {
        guard !columns.isEmpty else { return }
        let rows = shownRows
        if automatic { fittingAutomatically = true } else { fitTask?.cancel() }
        let tickets = columns.map { widths.begin($0) }
        let visible = table.rows(in: table.visibleRect)
        let start = visible.location == NSNotFound ? 0 : visible.location
        let inputs = columns.compactMap { column -> ColumnWidthMeasurement.Input? in
            guard let info = model.column(column) else { return nil }
            return ColumnWidthMeasurement.Input(
                column: column, name: info.name, type: info.displayType, text: info.valueCategory == .text,
                sample: texts.text(row: start, column: column))
        }
        let fontSize = presentation.style.fontSize
        let task = Task {
            var continueFitting = true
            defer {
                if automatic { fittingAutomatically = false }
                table.alphaValue = 1
                firstWidthsPending = false
                if automatic, continueFitting { fitVisibleColumns() }
            }
            do {
                let result = try await ColumnWidthMeasurement.measure(
                    rows: rows, columns: inputs, start: start, fontSize: fontSize, maximum: automatic ? 420 : 600)
                try Task.checkCancellation()
                guard shownRows === rows, presentation.style.fontSize == fontSize else {
                    for ticket in tickets { widths.abandon(ticket) }
                    return
                }
                applyingWidths = true
                defer { applyingWidths = false }
                for ticket in tickets where widths.accept(ticket) {
                    if let width = result[ticket.column] {
                        table.tableColumn(withIdentifier: identifier(for: ticket.column))?.width = width
                    }
                }
            } catch {
                for ticket in tickets { widths.abandon(ticket) }
                if shownRows === rows {
                    continueFitting = false
                    if !(error is CancellationError) { model.reportError(error, title: "Could Not Fit Columns") }
                }
            }
        }
        if automatic { automaticFitTask = task } else { fitTask = task }
    }
}
