import Foundation
import GewuKit

/// Opens throwaway files with the real engine. Each file is removed once read.
enum TestTables {
    static func session(_ contents: String, extension ext: String = "csv") async throws -> TableSession {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("gewu-test-\(UUID()).\(ext)")
        try contents.write(to: url, atomically: true, encoding: .utf8)
        defer { try? FileManager.default.removeItem(at: url) }
        return try await TableSession.open(url)
    }

    @MainActor
    static func model(_ contents: String, alerts: Alerts = Alerts()) async throws -> TableModel {
        let session = try await session(contents)
        return TableModel(url: URL(fileURLWithPath: session.source.info.path), session: session, alerts: alerts)
    }
}

extension TableSource {
    func id(_ name: String) -> ColumnID {
        columns.first { $0.name == name }!.id
    }
}

extension TableModel {
    func id(_ name: String) -> ColumnID { source.id(name) }
}

/// Waits for observable state without wall-clock sleeps.
@MainActor
func eventually(_ condition: @MainActor () -> Bool, timeout: Duration = .seconds(10)) async {
    let clock = ContinuousClock()
    let deadline = clock.now + timeout
    while !condition(), clock.now < deadline {
        await Task.yield()
        try? await Task.sleep(for: .milliseconds(1))
    }
}
