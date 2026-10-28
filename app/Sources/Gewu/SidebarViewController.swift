import AppKit

/// Left sidebar: file details and the list of columns.
/// Clicking a column scrolls the table to it.
final class SidebarViewController: NSViewController, NSTableViewDataSource, NSTableViewDelegate {
    var onSelectColumn: ((Int) -> Void)?

    private var columns: [ColumnInfo] = []
    private var filtered: Set<Int> = []

    private let fileDetails = NSStackView()
    private let list = NSTableView()

    private static let cellID = NSUserInterfaceItemIdentifier("sidebarColumn")

    override func loadView() {
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 240, height: 600))

        fileDetails.orientation = .vertical
        fileDetails.alignment = .leading
        fileDetails.spacing = 4

        let column = NSTableColumn(identifier: Self.cellID)
        list.addTableColumn(column)
        list.headerView = nil
        list.style = .sourceList
        list.rowHeight = 24
        list.backgroundColor = .clear
        list.dataSource = self
        list.delegate = self
        list.target = self
        list.action = #selector(columnClicked(_:))

        let scroll = NSScrollView()
        scroll.documentView = list
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true

        let fileHeader = Self.sectionLabel("File")
        let columnsHeader = Self.sectionLabel("Columns")

        for v in [fileHeader, fileDetails, columnsHeader, scroll] as [NSView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            root.addSubview(v)
        }
        let guide = root.safeAreaLayoutGuide
        NSLayoutConstraint.activate([
            fileHeader.topAnchor.constraint(equalTo: guide.topAnchor, constant: 10),
            fileHeader.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 18),

            fileDetails.topAnchor.constraint(equalTo: fileHeader.bottomAnchor, constant: 6),
            fileDetails.leadingAnchor.constraint(equalTo: fileHeader.leadingAnchor),
            fileDetails.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -14),

            columnsHeader.topAnchor.constraint(equalTo: fileDetails.bottomAnchor, constant: 16),
            columnsHeader.leadingAnchor.constraint(equalTo: fileHeader.leadingAnchor),

            scroll.topAnchor.constraint(equalTo: columnsHeader.bottomAnchor, constant: 4),
            scroll.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: root.bottomAnchor),
        ])
        view = root
    }

    func show(info: FileInfo) {
        let count = { (n: UInt64) in NumberFormatter.localizedString(from: NSNumber(value: n), number: .decimal) }
        let compression = Array(NSOrderedSet(array: info.compression.map { $0.lowercased() })) as? [String] ?? []
        let rows: [(String, String)] = [
            ("Rows", count(info.rowCount)),
            ("Columns", count(UInt64(info.columns.count))),
            ("Size", ByteCountFormatter.string(fromByteCount: Int64(info.fileSize), countStyle: .file)),
            ("Row groups", count(UInt64(info.rowGroups))),
            ("Compression", compression.isEmpty ? "—" : compression.joined(separator: ", ")),
        ]
        let grid = NSGridView(views: rows.map { key, value in
            let k = NSTextField(labelWithString: key)
            k.font = .systemFont(ofSize: 12)
            k.textColor = .secondaryLabelColor
            let v = NSTextField(labelWithString: value)
            v.font = .monospacedDigitSystemFont(ofSize: 12, weight: .regular)
            v.lineBreakMode = .byTruncatingTail
            v.toolTip = value
            v.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
            return [k, v]
        })
        grid.rowSpacing = 4
        grid.columnSpacing = 10
        fileDetails.arrangedSubviews.forEach { $0.removeFromSuperview() }
        fileDetails.addArrangedSubview(grid)

        columns = info.columns
        list.reloadData()
    }

    func setFiltered(_ columns: Set<Int>) {
        filtered = columns
        list.reloadData()
    }

    private static func sectionLabel(_ title: String) -> NSTextField {
        let label = NSTextField(labelWithString: title)
        label.font = .systemFont(ofSize: 11, weight: .semibold)
        label.textColor = .secondaryLabelColor
        return label
    }

    @objc private func columnClicked(_ sender: Any?) {
        let row = list.clickedRow
        if row >= 0 { onSelectColumn?(row) }
    }

    // MARK: - List

    func numberOfRows(in tableView: NSTableView) -> Int { columns.count }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let cell = (tableView.makeView(withIdentifier: Self.cellID, owner: self) as? SidebarColumnCell)
            ?? SidebarColumnCell(identifier: Self.cellID)
        let column = columns[row]
        cell.name.stringValue = column.name
        cell.type.stringValue = shortTypeName(column.typeName)
        cell.toolTip = "\(column.name): \(column.typeName)\(column.nullable ? " (nullable)" : "")"
        cell.funnel.isHidden = !filtered.contains(row)
        return cell
    }
}

private final class SidebarColumnCell: NSTableCellView {
    let name = NSTextField(labelWithString: "")
    let type = NSTextField(labelWithString: "")
    let funnel = NSImageView()

    init(identifier: NSUserInterfaceItemIdentifier) {
        super.init(frame: .zero)
        self.identifier = identifier
        name.font = .systemFont(ofSize: 13)
        name.lineBreakMode = .byTruncatingTail
        name.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        type.font = .systemFont(ofSize: 11)
        type.textColor = .secondaryLabelColor
        type.setContentHuggingPriority(.required, for: .horizontal)
        funnel.image = filterSymbol
        funnel.setContentHuggingPriority(.required, for: .horizontal)

        let stack = NSStackView(views: [name, funnel, type])
        stack.orientation = .horizontal
        stack.spacing = 6
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        textField = name
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 4),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            stack.centerYAnchor.constraint(equalTo: centerYAnchor),
            funnel.widthAnchor.constraint(equalToConstant: 13),
            funnel.heightAnchor.constraint(equalToConstant: 13),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }
}
