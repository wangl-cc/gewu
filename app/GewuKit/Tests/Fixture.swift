import Foundation
import GewuKit

/// A temporary file, removed when the fixture is released.
final class Fixture {
    let url: URL

    init(_ contents: String, extension ext: String = "csv") throws {
        url = FileManager.default.temporaryDirectory
            .appendingPathComponent("gewukit-\(UUID().uuidString).\(ext)")
        try contents.write(to: url, atomically: true, encoding: .utf8)
    }

    deinit { try? FileManager.default.removeItem(at: url) }

    func open() async throws -> TableSession {
        try await TableSession.open(url)
    }
}

extension TableSource {
    func id(_ name: String) -> ColumnID {
        columns.first { $0.name == name }!.id
    }
}

func exact(_ value: String) -> Filter {
    .condition(.exact(values: [value], exclude: false), includeNulls: false)
}
