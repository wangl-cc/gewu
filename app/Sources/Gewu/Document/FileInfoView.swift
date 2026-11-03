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

    init(info: DatasetInfo) {
        path = info.path
        var details = [
            Detail(title: "Kind", value: "\(info.format.displayName) table"),
            Detail(
                title: "Size",
                value: ByteCountFormatter.string(fromByteCount: Int64(clamping: info.fileSize), countStyle: .file)),
            Detail(title: "Rows", value: formatCount(info.rowCount)),
            Detail(title: "Columns", value: formatCount(UInt64(info.columns.count))),
        ]
        switch info.metadata {
        case .parquet(let rowGroups, let compression, let createdBy, let keyValues):
            details.append(Detail(title: "Row groups", value: formatCount(UInt64(rowGroups))))
            details.append(
                Detail(
                    title: "Compression",
                    value: compression.isEmpty ? "None" : compression.joined(separator: ", ")))
            if let createdBy {
                details.append(Detail(title: "Created by", value: createdBy))
            }
            metadata = keyValues.map { Detail(title: $0.key, value: $0.value) }
        case .delimited:
            metadata = []
        case .xlsx(let sheet, let sheets):
            details.append(Detail(title: "Worksheet", value: sheet))
            details.append(Detail(title: "Workbook sheets", value: sheets.joined(separator: ", ")))
            metadata = []
        }
        items = details
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
