import AppKit
import Foundation
import Testing

@Suite @MainActor
struct QuerySpecTests {
    private func model() throws -> DocumentModel {
        let path = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("gewu-query-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        try "a,b,c\n1,2,x\n1,1,y\n2,0,z\n".write(to: path, atomically: true, encoding: .utf8)
        return DocumentModel(url: path, document: try GewuSession.open(path: path.path))
    }

    private func settled(_ model: DocumentModel) async throws {
        for _ in 0..<500 {
            if !model.isBusy { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        Issue.record("Query did not settle")
    }

    @Test func sortingCyclesAndPrioritiesRemainIndependentOfDisplayOrder() async throws {
        let model = try model()
        defer { model.stop() }
        model.cycleSort(column: 0, multiple: false)
        model.cycleSort(column: 1, multiple: true)
        #expect(model.sortedColumns.map(\.column) == [0, 1])
        model.reorderColumns([2, 1, 0])
        #expect(model.sortedColumns.map(\.column) == [0, 1])
        model.cycleSort(column: 0, multiple: true)
        #expect(model.sortedColumns[0].sort?.descending == true)
        model.cycleSort(column: 0, multiple: true)
        #expect(model.sortedColumns.map(\.column) == [1])
        #expect(model.sortedColumns[0].sort?.priority == 0)
        model.cycleSort(column: 2, multiple: false)
        #expect(model.sortedColumns.map(\.column) == [2])
        model.clearSort()
        try await settled(model)
        #expect(model.appliedSpec.columns.map(\.column) == [2, 1, 0])
        #expect(
            try model.doc.rows(version: model.current.version, start: 0, count: 10, columns: [0]).sourceRows == [
                0, 1, 2,
            ])
    }

    @Test func confirmedHideClearsConditionsAndFindUsesRemainingColumns() async throws {
        let model = try model()
        defer { model.stop() }
        model.setFilter(Filter(condition: .exact(values: ["1"], exclude: false), includeNulls: false), column: 0)
        model.setSortDirection(column: 0, descending: true)
        try await settled(model)
        #expect(model.current.rowCount == 2)
        model.removeColumn(0)
        try await settled(model)
        #expect(model.current.rowCount == 3)
        #expect(model.sortedColumns.isEmpty)
        #expect(model.appliedFilterColumns.isEmpty)
        model.find.query = "1"
        model.updateFind()
        for _ in 0..<500 {
            if model.find.hasMatches != nil { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        #expect(model.find.currentMatch?.column == 1)
        model.showColumn(0)
        #expect(model.spec.columns.last?.column == 0)
        #expect(model.spec.columns.last?.filter == nil)
        #expect(model.spec.columns.last?.sort == nil)
        try await settled(model)
    }

    @Test func shiftHeaderClickUsesExplicitMultiSortCallback() throws {
        let table = NSTableView(frame: NSRect(x: 0, y: 0, width: 120, height: 100))
        let column = NSTableColumn(identifier: .init("value"))
        column.width = 120
        table.addTableColumn(column)
        let header = FilterHeaderView(frame: NSRect(x: 0, y: 0, width: 120, height: 32))
        table.headerView = header
        var clicked: Int?
        header.onShiftSort = { clicked = $0 }
        let event = try #require(
            NSEvent.mouseEvent(
                with: .leftMouseDown, location: NSPoint(x: 20, y: 10), modifierFlags: .shift,
                timestamp: 0, windowNumber: 0, context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
        header.mouseDown(with: event)
        #expect(clicked == 0)
    }

    @Test func exactDraftPreservesBytesAndNullsAreIndependent() {
        let composed = LiteralTextValue(value: "é")
        let decomposed = LiteralTextValue(value: "e\u{301}")
        #expect(composed != decomposed)
        var draft = ExactFilterDraft(nil)
        draft.set(composed, selected: false)
        #expect(!draft.contains(composed) && draft.contains(decomposed))
        let filter = NullMode.exclude.applying(to: draft.condition)
        #expect(filter?.includeNulls == false)
        if case .exact(let values, let exclude) = filter?.condition {
            #expect(exclude && values.count == 1)
        } else {
            Issue.record("Expected exact selection")
        }
        let onlyNull = NullMode.only.applying(to: draft.condition)
        #expect(onlyNull?.includeNulls == true)
        if case .exact(let values, let exclude) = onlyNull?.condition {
            #expect(values.isEmpty && !exclude)
        } else {
            Issue.record("Expected null-only selection")
        }
        draft.all(true)
        #expect(NullMode.include.applying(to: draft.condition) == nil)
    }

    @Test func failedReplacementRestoresAViewCommittedBeforeCancellation() async throws {
        let model = try model()
        defer { model.stop() }
        let accepted = QuerySpec(columns: [
            ColSpec(
                column: 0, sort: nil,
                filter: Filter(condition: .exact(values: ["1"], exclude: false), includeNulls: false))
        ])
        // Rust committed the obsolete operation before Swift could publish it.
        let operation = try model.doc.prepareView(spec: accepted)
        let committed = try operation.run()
        operation.cancel()
        model.setFilter(Filter(condition: .range(min: "invalid", max: nil), includeNulls: false), column: 0)
        try await settled(model)
        #expect(model.errorMessage != nil)
        #expect(model.current.version == committed.version)
        #expect(model.appliedSpec == accepted && model.spec == accepted)
        #expect(try model.doc.rows(version: model.current.version, start: 0, count: 3, columns: [0]).nRows == 2)
    }

    @Test func cancellingCandidateRequestDoesNotCloseReplacement() async throws {
        let document = try model()
        defer { document.stop() }
        let choices = TextChoicesModel()
        defer { choices.cancel() }
        let old = Task {
            await choices.load(document: document.doc, column: 2, querySpec: document.spec, query: "x", after: nil)
        }
        await Task.yield()
        old.cancel()
        choices.pause()
        await choices.load(document: document.doc, column: 2, querySpec: document.spec, query: "y", after: nil)
        await old.value
        #expect(choices.page?.values.map(\.value) == ["y"])
        #expect(!choices.loading && choices.error == nil)
    }
}
