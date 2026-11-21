import Foundation
import GewuKit
import Synchronization
import Testing

/// Exercises the real window model and engine without creating windows or
/// modifying recent documents. The fixture has distinct schemas, four valid
/// sheets, and one sheet containing an Excel error.
@Suite(.serialized) @MainActor
struct WorkbookLifecycleTests {
    private static func fixture(_ name: String = "multi-sheet") throws -> URL {
        try #require(
            Bundle(for: FixtureBundle.self).url(forResource: name, withExtension: "xlsx", subdirectory: "Fixtures"))
    }

    private static func settled(_ model: WindowModel) async {
        await eventually { !model.isOpening }
    }

    private static func firstCell(_ model: WindowModel) async throws -> String? {
        let table = try #require(model.table)
        return try await table.displayed.text(of: table.source.columns[0].id, rows: 0..<1).cell(0)
    }

    @Test static func unicodeWorksheetNamesRemainDistinct() async throws {
        let composed = WorksheetName(rawValue: "Caf\u{E9}")
        let decomposed = WorksheetName(rawValue: "Cafe\u{301}")
        let probe = OpenProbe(slowSheet: composed.rawValue)
        let model = WindowModel(url: try fixture("unicode-sheets"), loader: probe.loader, didOpen: { _ in })
        defer { model.stop() }
        model.open()
        await settled(model)
        _ = try #require(model.table)
        #expect(model.sheets.count == 3)
        #expect(Set(model.sheets).count == 3, "Worksheet identities must distinguish the two encodings")

        model.selectSheet(composed)
        await probe.waitForSlowStart()
        model.selectSheet(decomposed)
        #expect(model.pendingSheet == decomposed)
        probe.releaseSlow()
        await settled(model)
        #expect(model.selectedSheet == decomposed)
        #expect(try await firstCell(model) == "decomposed")

        model.selectSheet(composed)
        probe.releaseSlow()
        await settled(model)
        #expect(model.selectedSheet == composed)
        #expect(try await firstCell(model) == "composed")
    }

    @Test static func catalogSurvivesABadFirstSheet() async throws {
        let live = TableLoader.live
        let loader = TableLoader(
            worksheetNames: { url in
                let names = try await live.worksheetNames(url)
                return ["Bad"] + names.filter { $0 != "Bad" }
            }, open: live.open)
        let model = WindowModel(url: try fixture(), loader: loader, didOpen: { _ in })
        defer { model.stop() }
        model.open()
        await settled(model)
        #expect(model.table == nil)
        #expect(model.sheets.first?.rawValue == "Bad" && model.sheets.contains(.init(rawValue: "First")))
        #expect(model.openError?.contains("#DIV/0!") == true)
        #expect(model.alerts.current?.title == "Could Not Open Table")
        model.selectSheet(.init(rawValue: "First"))
        await settled(model)
        #expect(model.selectedSheet?.rawValue == "First")
        #expect(model.table?.info.columns.first?.name == "column_0")
    }

    @Test static func switchingUsesTheLatestRequest() async throws {
        let probe = OpenProbe()
        let model = WindowModel(url: try fixture(), loader: probe.loader, didOpen: { _ in })
        model.open()
        await settled(model)
        let original = try #require(model.table)
        #expect(model.selectedSheet?.rawValue == "First")
        original.find.query = "keep while loading"

        model.selectSheet(.init(rawValue: "Bad"))
        await settled(model)
        #expect(model.table === original, "A failed switch keeps the loaded table")
        #expect(original.find.query == "keep while loading")

        model.selectSheet(.init(rawValue: "Slow"))
        await probe.waitForSlowStart()
        #expect(model.table === original && model.pendingSheet?.rawValue == "Slow")
        model.selectSheet(.init(rawValue: "Queued"))
        await Task.yield()
        model.selectSheet(.init(rawValue: "Last"))
        probe.releaseSlow()
        await settled(model)
        #expect(model.selectedSheet?.rawValue == "Last")
        #expect(model.table !== original && model.table?.info.columns.first?.name == "column_3")
        #expect(model.table?.find.query == "" && model.table?.chartX == nil)
        #expect(!probe.openedSheets.contains("Queued"), "A replaced open that has not started never reads")

        // Selecting the shown sheet cancels a pending switch.
        let last = model.table
        model.selectSheet(.init(rawValue: "Slow"))
        await probe.waitForSlowStart()
        model.selectSheet(.init(rawValue: "Last"))
        #expect(!model.isOpening && model.pendingSheet == nil)
        probe.releaseSlow()
        // A later open waits for the read in progress, then runs.
        model.selectSheet(.init(rawValue: "First"))
        await settled(model)
        #expect(model.selectedSheet?.rawValue == "First" && model.table !== last)

        // Stopping during an open keeps the table and publishes nothing later.
        let retained = model.table
        model.selectSheet(.init(rawValue: "Slow"))
        await probe.waitForSlowStart()
        model.stop()
        probe.releaseSlow()
        await probe.waitForSlowFinish()
        for _ in 0..<20 { await Task.yield() }
        #expect(model.table === retained && !model.isOpening)
    }

    @Test static func headerOptionReloadsData() async throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let csv = folder.appendingPathComponent("headerless.csv")
        try "1,Alice\n2,Bob\n".write(to: csv, atomically: true, encoding: .utf8)
        let model = WindowModel(url: csv, hasHeader: false, didOpen: { _ in })
        defer { model.stop() }
        model.open()
        await settled(model)
        #expect(model.table?.displayed.count == 2 && !model.hasHeader)
        #expect(try await firstCell(model) == "1")
        model.reopen(hasHeader: true)
        await settled(model)
        #expect(model.table?.displayed.count == 1 && model.hasHeader)
        #expect(model.table?.info.columns.first?.name == "1")
        let retained = model.table
        try FileManager.default.removeItem(at: csv)
        model.reopen(hasHeader: false)
        await settled(model)
        #expect(model.table === retained && model.hasHeader, "A failed reload keeps the table and its options")

        let workbook = WindowModel(url: try fixture(), hasHeader: false, didOpen: { _ in })
        defer { workbook.stop() }
        workbook.open()
        await settled(workbook)
        #expect(!workbook.hasHeader && workbook.table?.info.columns.first?.name == "column_1")
        workbook.selectSheet(.init(rawValue: "Last"))
        await settled(workbook)
        #expect(!workbook.hasHeader && workbook.selectedSheet?.rawValue == "Last")
        #expect(try await firstCell(workbook) == "column_3", "Worksheet changes keep the header option")
    }

    @Test static func inspectorSettingsSurviveWorksheetChanges() async throws {
        let model = WindowModel(url: try fixture(), didOpen: { _ in })
        defer { model.stop() }
        model.open()
        await settled(model)
        model.inspectorSettings.histogramBins = 100
        model.inspectorPage = .chart
        model.selectSheet(.init(rawValue: "Last"))
        await settled(model)
        #expect(model.inspectorSettings.histogramBins == 100 && model.inspectorPage == .chart)
    }

    @Test static func analyzingAColumnShowsTheInspectorOnItsColumnPage() async throws {
        let defaults = try #require(UserDefaults(suiteName: "gewu-analyze-\(UUID())"))
        let model = WindowModel(url: try fixture(), defaults: defaults, didOpen: { _ in })
        defer { model.stop() }
        model.open()
        await settled(model)
        let table = try #require(model.table)
        model.showsInspector = false
        model.inspectorPage = .chart
        let column = try #require(table.visibleColumns.last)
        table.inspect(table.visibleColumns[0])
        model.inspect(column)
        #expect(model.showsInspector && model.inspectorPage == .column && table.inspectedColumn == column)
    }
}

/// Opens worksheets with the real engine, holding one named sheet until released.
private final class OpenProbe: Sendable {
    private let opened = Mutex<[String]>([])
    private let slowStarted = DispatchSemaphore(value: 0)
    private let slowReleased = DispatchSemaphore(value: 0)
    private let slowFinished = DispatchSemaphore(value: 0)
    private let slowSheet: String

    init(slowSheet: String = "Slow") {
        self.slowSheet = slowSheet
    }

    var openedSheets: [String] { opened.withLock { $0 } }

    var loader: TableLoader {
        TableLoader(
            worksheetNames: { try await TableSession.worksheetNames($0) },
            open: { url, options in
                let sheet = options.sheet
                self.opened.withLock { $0.append(sheet ?? "First") }
                if let sheet, sheet.utf8.elementsEqual(self.slowSheet.utf8) {
                    self.slowStarted.signal()
                    await Self.wait(self.slowReleased)
                    defer { self.slowFinished.signal() }
                    return try await TableSession.open(url, options: options)
                }
                return try await TableSession.open(url, options: options)
            })
    }

    private static func wait(_ semaphore: DispatchSemaphore) async {
        await withCheckedContinuation { continuation in
            DispatchQueue.global().async {
                semaphore.wait()
                continuation.resume()
            }
        }
    }

    func waitForSlowStart() async { await Self.wait(slowStarted) }
    func releaseSlow() { slowReleased.signal() }
    func waitForSlowFinish() async { await Self.wait(slowFinished) }
}

private final class FixtureBundle {}
