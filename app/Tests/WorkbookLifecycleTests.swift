import Combine
import Foundation
import Testing

/// Exercises the real window/sheet models and FFI without creating application
/// windows or modifying recent documents. The fixture has distinct schemas,
/// four valid sheets, and one sheet containing an Excel error.
@Suite(.serialized)
struct WorkbookLifecycleTests {
    private static func fixture(_ name: String = "multi-sheet") throws -> URL {
        try #require(
            Bundle(for: FixtureBundle.self).url(
                forResource: name, withExtension: "xlsx", subdirectory: "Fixtures"))
    }

    @Test
    @MainActor
    static func unicodeWorksheetNamesRemainDistinct() async throws {
        let composed = WorksheetName(rawValue: "Caf\u{E9}")
        let decomposed = WorksheetName(rawValue: "Cafe\u{301}")
        let probe = OpenProbe(slowSheet: composed.rawValue)
        let model = WorkbookModel(
            url: try fixture("unicode-sheets"),
            openDocument: { path, sheet, _ in try probe.open(path: path, sheet: sheet) },
            didOpen: { _ in })
        defer { model.stop() }
        model.open()
        await settled(model)
        _ = try #require(model.document)
        #expect(model.sheets.count == 3)
        #expect(Set(model.sheets).count == 3, "Worksheet bar identities must distinguish the two encodings")

        model.selectSheet(composed)
        await probe.waitForSlowStart()
        model.selectSheet(decomposed)
        #expect(model.pendingSheet == decomposed)
        probe.releaseSlow()
        await settled(model)
        #expect(model.selectedSheet == decomposed)
        let decomposedDocument = try #require(model.document)
        let decomposedRows = try decomposedDocument.doc.rows(version: 0, start: 0, count: 1, columns: [0])
        #expect(decomposedRows.cells == ["decomposed"])

        model.selectSheet(composed)
        // Always release: the buggy implementation skips this open entirely.
        probe.releaseSlow()
        await settled(model)
        #expect(model.selectedSheet == composed)
        let composedDocument = try #require(model.document)
        let composedRows = try composedDocument.doc.rows(version: 0, start: 0, count: 1, columns: [0])
        #expect(composedRows.cells == ["composed"])
    }

    @Test
    @MainActor
    static func catalogSurvivesBadFirstSheet() async throws {
        let fixture = try fixture()
        let model = WorkbookModel(
            url: fixture,
            sheetNames: { path in
                let names = try xlsxSheetNames(path: path)
                return ["Bad"] + names.filter { $0 != "Bad" }
            },
            didOpen: { _ in })
        model.open()
        await settled(model)
        #expect(model.document == nil)
        #expect(model.sheets.first?.rawValue == "Bad" && model.sheets.contains(.init(rawValue: "First")))
        #expect(model.errorMessage?.contains("#DIV/0!") == true)
        model.selectSheet(.init(rawValue: "First"))
        await settled(model)
        #expect(model.selectedSheet?.rawValue == "First")
        #expect(model.document?.info.columns.first?.name == "column_0")
        model.stop()
    }

    @Test
    @MainActor
    static func switchingUsesLatestRequest() async throws {
        let fixture = try fixture()
        let probe = OpenProbe()
        let model = WorkbookModel(
            url: fixture, openDocument: { path, sheet, _ in try probe.open(path: path, sheet: sheet) },
            didOpen: { _ in })
        model.open()
        await settled(model)
        let original = try #require(model.document)
        #expect(model.selectedSheet?.rawValue == "First")
        original.searchText = "keep while loading"
        original.chartY = 0

        model.selectSheet(.init(rawValue: "Bad"))
        await settled(model)
        #expect(model.document === original, "A failed switch must preserve the loaded document")
        #expect(original.searchText == "keep while loading")

        model.selectSheet(.init(rawValue: "Slow"))
        await probe.waitForSlowStart()
        #expect(model.document === original && model.pendingSheet?.rawValue == "Slow")
        model.selectSheet(.init(rawValue: "Queued"))
        await Task.yield()
        model.selectSheet(.init(rawValue: "Last"))
        probe.releaseSlow()
        await settled(model)
        #expect(model.selectedSheet?.rawValue == "Last")
        #expect(model.document !== original)
        #expect(model.document?.info.columns.first?.name == "column_3")
        #expect(model.document?.searchText == "" && model.document?.chartY == nil)
        #expect(!probe.openedSheets.contains("Queued"), "An obsolete queued open must not decode")
        #expect(model.document?.current.version == 0 && original.current.version == 0)

        let last = model.document
        model.selectSheet(.init(rawValue: "Slow"))
        await probe.waitForSlowStart()
        model.selectSheet(.init(rawValue: "Last"))
        #expect(!model.isOpening && model.pendingSheet == nil)
        probe.releaseSlow()
        // A later real open is a barrier behind the obsolete read on the serial queue.
        model.selectSheet(.init(rawValue: "First"))
        await settled(model)
        #expect(model.selectedSheet?.rawValue == "First" && model.document !== last)

        let other = WorkbookModel(url: fixture, didOpen: { _ in })
        model.showsInspector = true
        other.showsInspector = false
        #expect(model.showsInspector && !other.showsInspector)

        let retained = model.document
        model.selectSheet(.init(rawValue: "Slow"))
        await probe.waitForSlowStart()
        model.stop()
        probe.releaseSlow()
        // Drain MainActor and the synchronous read without a wall-clock sleep.
        await probe.waitForSlowFinish()
        await Task.yield()
        #expect(model.document === retained && !model.isOpening)
    }

    @Test
    @MainActor
    static func headerOptionReloadsData() async throws {
        let fixture = try fixture()
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let csv = folder.appendingPathComponent("headerless.csv")
        try "1,Alice\n2,Bob\n".write(to: csv, atomically: true, encoding: .utf8)
        let model = WorkbookModel(url: csv, hasHeader: false, didOpen: { _ in })
        model.open()
        await settled(model)
        #expect(model.document?.current.rowCount == 2 && !model.hasHeader)
        let first = try model.document?.doc.rows(version: 0, start: 0, count: 1, columns: [0])
        #expect(first?.cells == ["1"])
        try await presentationContracts(#require(model.document))
        model.reopen(hasHeader: true)
        await settled(model)
        #expect(model.document?.current.rowCount == 1 && model.hasHeader)
        #expect(model.document?.info.columns.first?.name == "1")
        let retained = model.document
        try FileManager.default.removeItem(at: csv)
        model.reopen(hasHeader: false)
        await settled(model)
        #expect(
            model.document === retained && model.hasHeader, "A failed reload preserves the document and its options")

        let workbook = WorkbookModel(url: fixture, hasHeader: false, didOpen: { _ in })
        workbook.open()
        await settled(workbook)
        #expect(!workbook.hasHeader && workbook.document?.info.columns.first?.name == "column_1")
        workbook.selectSheet(.init(rawValue: "Last"))
        await settled(workbook)
        #expect(!workbook.hasHeader && workbook.selectedSheet?.rawValue == "Last")
        let rows = try #require(workbook.document).doc.rows(version: 0, start: 0, count: 1, columns: [0])
        #expect(rows.cells == ["column_3"], "Worksheet changes must retain the header option")
    }

    @MainActor
    static func presentationContracts(_ document: DocumentModel) async throws {
        let chart = ScatterModel()
        document.selectedSourceRow = 0
        for base in [0, 1] {
            await chart.load(
                document: document.doc, version: document.current.version,
                x: nil, y: 0, xKind: nil, yKind: .integer, rowNumberBase: base)
            let point = try #require(chart.result).points[0]
            #expect(point.sourceRow == 0, "Presentation must not change source row identity")
            let label = formatRowNumber(point.sourceRow, base: base)
            #expect(label == String(base) && point.xLabel == label && point.x == Double(base))
            #expect(document.status(rowNumberBase: base).contains("row \(label) in file"))
        }
        document.selectedSourceRow = nil
        do {
            _ = try document.doc.parseBound(column: 0, text: "not a number")
            Issue.record("Invalid numeric bounds must fail")
        } catch GewuError.InvalidArgument(let message) {
            let error: Error = GewuError.InvalidArgument(message: message)
            #expect(error.userMessage == message, "Show the Core message, not the generated enum description")
            document.reportError(error)
            #expect(document.errorMessage == message)
            document.isErrorPresented = false
        }
    }

    @MainActor
    static func settled(_ model: WorkbookModel) async {
        guard model.isOpening else { return }
        var observation: AnyCancellable?
        await withCheckedContinuation { continuation in
            observation = model.$isOpening.filter { !$0 }.prefix(1).sink { _ in continuation.resume() }
        }
        observation?.cancel()
    }
}

private final class OpenProbe: @unchecked Sendable {
    private let lock = NSLock()
    private var opened: [String] = []
    private let slowStarted = DispatchSemaphore(value: 0)
    private let slowReleased = DispatchSemaphore(value: 0)
    private let slowFinished = DispatchSemaphore(value: 0)
    private let slowSheet: String

    init(slowSheet: String = "Slow") {
        self.slowSheet = slowSheet
    }

    var openedSheets: [String] {
        lock.lock()
        defer { lock.unlock() }
        return opened
    }

    func open(path: String, sheet: String?) throws -> GewuDocument {
        lock.lock()
        opened.append(sheet ?? "First")
        lock.unlock()
        if let sheet, sheet.utf8.elementsEqual(slowSheet.utf8) {
            slowStarted.signal()
            slowReleased.wait()
            defer { slowFinished.signal() }
            return try GewuDocument.openSheet(path: path, sheet: sheet)
        }
        if let sheet { return try GewuDocument.openSheet(path: path, sheet: sheet) }
        return try GewuDocument.open(path: path)
    }

    func waitForSlowStart() async {
        await withCheckedContinuation { continuation in
            DispatchQueue.global().async {
                self.slowStarted.wait()
                continuation.resume()
            }
        }
    }
    func releaseSlow() { slowReleased.signal() }
    func waitForSlowFinish() async {
        await withCheckedContinuation { continuation in
            DispatchQueue.global().async {
                // Earlier Slow requests are also complete; consume their signals.
                for _ in 0..<3 { self.slowFinished.wait() }
                continuation.resume()
            }
        }
    }
}

private final class FixtureBundle {}
