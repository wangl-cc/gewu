import Foundation
import Testing

/// Tests App table and copy behavior through the real Core bindings.
@Suite
struct TableExecutionTests {
    static func block(_ values: [String?], start: Int = 0, columns: Int = 1) -> RowBlock {
        RowBlock(
            version: 1, start: UInt64(start), nRows: UInt32(values.count / columns), nCols: UInt32(columns),
            cells: values, sourceRows: (0..<(values.count / columns)).map(UInt64.init))
    }

    @Test
    static func cacheBudgets() throws {
        let cache = RowCache(byteLimit: 256, cellLimit: 3)
        let a = RowCache.Key(start: 0, column: 0)
        let b = RowCache.Key(start: 0, column: 1)
        let c = RowCache.Key(start: 64, column: 0)
        #expect(cache.insert(block(["first", nil]), for: a))
        #expect(cache.insert(block(["second"]), for: b))
        #expect(cache.block(for: a)?.cells == ["first", nil])
        #expect(cache.insert(block(["third"]), for: c))
        #expect(cache.block(for: b) == nil, "Evict least recently used column block")
        #expect(cache.block(for: a) != nil, "Same row block in another column must stay distinct")
        #expect(
            !cache.insert(block([String(repeating: "界", count: 100)]), for: b), "Use UTF-8 bytes, not character count")
        #expect(cache.byteCount <= 256 && cache.cellCount <= 3)
        let metadata = RowBlock(
            version: 1, start: 0, nRows: 4, nCols: 0, cells: [], sourceRows: [0, 1, 2, 3])
        #expect(!cache.insert(metadata, for: .init(start: 0, column: nil)), "Source-only blocks consume cell budget")
        cache.clear()
        #expect(cache.byteCount == 0 && cache.cellCount == 0 && cache.block(for: a) == nil)
    }

    @Test
    static func copyOrderAndEscaping() throws {
        let columns = Array((0..<130).reversed()).map(UInt32.init)
        let ranges = [2..<4, 100..<165]
        var requests = 0
        let result = try CopyText.format(rows: ranges, columns: columns, names: columns.map { "h\($0)" }) {
            start, count, selected in
            #expect(count <= 64 && selected.count <= 64)
            requests += 1
            return block(
                (start..<(start + count)).flatMap { row in selected.map { "\(row):\($0)" } },
                start: start, columns: selected.count)
        }
        let lines = result.split(separator: "\n")
        #expect(lines.count == 68 && requests == 9)
        #expect(lines[1].split(separator: "\t").first == "2:129")
        #expect(lines.last?.split(separator: "\t").last == "164:0")
        let quoted = try CopyText.format(rows: [0..<1], columns: [0, 1, 2, 3], names: ["a", "b", "c", "d"]) { _, _, _ in
            block(["tab\tvalue", "line\nvalue", "a\"b", nil], columns: 4)
        }
        #expect(quoted == "a\tb\tc\td\n\"tab\tvalue\"\t\"line\nvalue\"\t\"a\"\"b\"\t")
        let withoutHeader = try CopyText.format(
            rows: [0..<2], columns: [0], names: ["value"], includeHeaders: false
        ) { _, _, _ in
            block([nil, "second"])
        }
        #expect(withoutHeader == "\nsecond", "An empty first data row must survive when headers are excluded")
    }

    @Test
    static func copyCancellation() async throws {
        let task = Task.detached {
            withUnsafeCurrentTask { $0?.cancel() }
            return try CopyText.format(rows: [0..<1_000_000], columns: [0], names: ["a"]) { _, _, _ in
                Issue.record("Cancelled copy must not fetch")
                throw CancellationError()
            }
        }
        do {
            _ = try await task.value
            Issue.record("Expected cancellation")
        } catch is CancellationError {}
    }

    @Test
    static func liveVersionChecks() async throws {
        let path = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("gewu-table-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        let csv = "value,label\n" + (0..<130).map { "\($0),row\($0)" }.joined(separator: "\n")
        try csv.write(to: path, atomically: true, encoding: .utf8)
        let doc = try GewuDocument.open(path: path.path)
        let version = doc.currentView().version
        let reader = TableRowReader()
        let source = try await reader.read(doc, version: version, start: 2, count: 4, columns: [])
        #expect(source.cells.isEmpty && source.sourceRows == [2, 3, 4, 5])
        var calls = 0
        do {
            _ = try CopyText.format(rows: [0..<130], columns: [1, 0], names: ["label", "value"]) {
                start, count, columns in
                calls += 1
                if calls == 2 {
                    _ = try doc.applyView(
                        spec: QuerySpec(sort: [SortKey(column: 0, descending: true)], filters: [:]))
                }
                return try doc.rows(version: version, start: UInt64(start), count: UInt32(count), columns: columns)
            }
            Issue.record("Copy must fail when a later batch sees a different version")
        } catch GewuError.StaleVersion {}
        #expect(calls == 2)
        do {
            _ = try await reader.read(doc, version: version, start: 0, count: 64, columns: [0])
            Issue.record("Old table requests must not return rows from the new version")
        } catch GewuError.StaleVersion {}
        let work = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            return try await reader.read(doc, version: doc.currentView().version, start: 0, count: 64, columns: [0])
        }
        do {
            _ = try await work.value
            Issue.record("Expected cancelled table read")
        } catch is CancellationError {}
    }
}
