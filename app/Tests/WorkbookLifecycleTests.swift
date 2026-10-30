import Combine
import Foundation

/// Exercises the real window/sheet models and FFI without creating application
/// windows or modifying recent documents. The fixture has distinct schemas,
/// four valid sheets, and one sheet containing an Excel error.
@main
struct WorkbookLifecycleTests {
    @MainActor
    static func main() async throws {
        ChartPresentationTests.run()
        FileOpeningTests.run()
        let fixture = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().appending(path: "Fixtures/multi-sheet.xlsx")
        try await catalogSurvivesBadFirstSheet(fixture)
        try await switchingUsesLatestRequest(fixture)
        try await headerOptionReloadsData(fixture)
        print("Workbook lifecycle regressions passed")
    }

    @MainActor
    static func catalogSurvivesBadFirstSheet(_ fixture: URL) async throws {
        let model = WorkbookModel(
            url: fixture,
            sheetNames: { path in
                let names = try xlsxSheetNames(path: path)
                return ["Bad"] + names.filter { $0 != "Bad" }
            },
            didOpen: { _ in })
        model.open()
        await settled(model)
        assert(model.document == nil)
        assert(model.sheets.first == "Bad" && model.sheets.contains("First"))
        assert(model.errorMessage?.contains("#DIV/0!") == true)
        model.selectSheet("First")
        await settled(model)
        assert(model.selectedSheet == "First")
        assert(model.document?.info?.columns.first?.name == "column_0")
        model.stop()
    }

    @MainActor
    static func switchingUsesLatestRequest(_ fixture: URL) async throws {
        let probe = OpenProbe()
        let model = WorkbookModel(
            url: fixture, openDocument: { path, sheet, _ in try probe.open(path: path, sheet: sheet) }, didOpen: { _ in })
        model.open()
        await settled(model)
        let original = model.document!
        assert(model.selectedSheet == "First")
        original.searchText = "keep while loading"
        original.chartY = 0

        model.selectSheet("Bad")
        await settled(model)
        assert(model.document === original, "A failed switch must preserve the loaded document")
        assert(original.searchText == "keep while loading")

        model.selectSheet("Slow")
        await probe.waitForSlowStart()
        assert(model.document === original && model.pendingSheet == "Slow")
        model.selectSheet("Queued")
        await Task.yield()
        model.selectSheet("Last")
        probe.releaseSlow()
        await settled(model)
        assert(model.selectedSheet == "Last")
        assert(model.document !== original)
        assert(model.document?.info?.columns.first?.name == "column_3")
        assert(model.document?.searchText == "" && model.document?.chartY == nil)
        assert(!probe.openedSheets.contains("Queued"), "An obsolete queued open must not decode")
        assert(model.document?.current.version == 0 && original.current.version == 0)

        let last = model.document
        model.selectSheet("Slow")
        await probe.waitForSlowStart()
        model.selectSheet("Last")
        assert(!model.isOpening && model.pendingSheet == nil)
        probe.releaseSlow()
        // A later real open is a barrier behind the obsolete read on the serial queue.
        model.selectSheet("First")
        await settled(model)
        assert(model.selectedSheet == "First" && model.document !== last)

        let other = WorkbookModel(url: fixture, didOpen: { _ in })
        model.showsInspector = true
        other.showsInspector = false
        assert(model.showsInspector && !other.showsInspector)

        let retained = model.document
        model.selectSheet("Slow")
        await probe.waitForSlowStart()
        model.stop()
        probe.releaseSlow()
        // Drain MainActor and the synchronous read without a wall-clock sleep.
        await probe.waitForSlowFinish()
        await Task.yield()
        assert(model.document === retained && !model.isOpening)
    }

    @MainActor
    static func headerOptionReloadsData(_ fixture: URL) async throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let csv = folder.appendingPathComponent("headerless.csv")
        try "1,Alice\n2,Bob\n".write(to: csv, atomically: true, encoding: .utf8)
        let model = WorkbookModel(url: csv, hasHeader: false, didOpen: { _ in })
        model.open()
        await settled(model)
        assert(model.document?.current.rowCount == 2 && !model.hasHeader)
        let first = try model.document?.doc?.rows(version: 0, start: 0, count: 1, columns: [0])
        assert(first?.cells == ["1"])
        model.reopen(hasHeader: true)
        await settled(model)
        assert(model.document?.current.rowCount == 1 && model.hasHeader)
        assert(model.document?.info?.columns.first?.name == "1")
        let retained = model.document
        try FileManager.default.removeItem(at: csv)
        model.reopen(hasHeader: false)
        await settled(model)
        assert(model.document === retained && model.hasHeader, "A failed reload preserves the document and its options")

        let workbook = WorkbookModel(url: fixture, hasHeader: false, didOpen: { _ in })
        workbook.open()
        await settled(workbook)
        assert(!workbook.hasHeader && workbook.document?.info?.columns.first?.name == "column_1")
        workbook.selectSheet("Last")
        await settled(workbook)
        assert(!workbook.hasHeader && workbook.selectedSheet == "Last")
        let rows = try workbook.document!.doc!.rows(version: 0, start: 0, count: 1, columns: [0])
        assert(rows.cells == ["column_3"], "Worksheet changes must retain the header option")
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

    var openedSheets: [String] {
        lock.lock()
        defer { lock.unlock() }
        return opened
    }

    func open(path: String, sheet: String?) throws -> GewuDocument {
        lock.lock()
        opened.append(sheet ?? "First")
        lock.unlock()
        if sheet == "Slow" {
            slowStarted.signal()
            slowReleased.wait()
            defer { slowFinished.signal() }
            return try GewuDocument.openSheet(path: path, sheet: "Slow")
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
