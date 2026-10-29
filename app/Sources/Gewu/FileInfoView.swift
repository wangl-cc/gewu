import SwiftUI

/// Small, immutable window value; opening an info window never reopens the data.
struct FileDetails: Codable, Hashable {
    struct Detail: Codable, Hashable {
        let title: String
        let value: String
    }

    let path: String
    let items: [Detail]
    let metadata: [Detail]

    init(info: FileInfo) {
        path = info.path
        items = [
            Detail(title: "Kind", value: "Parquet table"),
            Detail(title: "Size", value: ByteCountFormatter.string(fromByteCount: Int64(clamping: info.fileSize), countStyle: .file)),
            Detail(title: "Rows", value: formatCount(info.rowCount)),
            Detail(title: "Columns", value: formatCount(UInt64(info.columns.count))),
            Detail(title: "Row groups", value: formatCount(UInt64(info.rowGroups))),
            Detail(title: "Compression", value: info.compression.isEmpty ? "None" : info.compression.joined(separator: ", ")),
        ] + (info.createdBy.map { [Detail(title: "Created by", value: $0)] } ?? [])
        metadata = info.metadata.map { Detail(title: $0.key, value: $0.value) }
    }
}

struct FileInfoView: View {
    let details: FileDetails
    private var url: URL { URL(fileURLWithPath: details.path) }

    var body: some View {
        Form {
            Section {
                LabeledContent("Name", value: url.lastPathComponent)
                ForEach(Array(details.items.enumerated()), id: \.offset) { _, item in
                    LabeledContent(item.title, value: item.value)
                }
                LabeledContent("Location") {
                    Text(details.path).multilineTextAlignment(.trailing).help(details.path)
                }
                Button("Show in Finder") { NSWorkspace.shared.activateFileViewerSelecting([url]) }
            }
            if !details.metadata.isEmpty {
                Section("Metadata") {
                    ForEach(Array(details.metadata.enumerated()), id: \.offset) { _, item in
                        LabeledContent(item.title) {
                            Text(item.value).multilineTextAlignment(.trailing)
                        }
                    }
                }
            }
        }
        .formStyle(.grouped)
        .textSelection(.enabled)
        .frame(minWidth: 400, minHeight: 320)
        .navigationTitle("\(url.lastPathComponent) Info")
    }
}
