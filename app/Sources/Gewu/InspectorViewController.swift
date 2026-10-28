import AppKit

/// Right-side inspector: what the table itself doesn't show. A searchable
/// column list to jump around, then statistics for the selected column over
/// the current view (filters and search apply), then file details.
final class InspectorViewController: NSViewController, NSTableViewDataSource, NSTableViewDelegate,
    NSSearchFieldDelegate
{
    /// Called when a column is chosen in the list.
    var onSelectColumn: ((Int) -> Void)?

    private var doc: GewuDocument?
    private var info: FileInfo?
    private var kinds: [ColumnKind] = []
    private var filtered: Set<Int> = []
    private var selected: Int?
    private var state: ViewState?
    private var generation = 0

    private let columnSearch = NSSearchField()
    private let columnList = NSTableView()
    private var shownColumns: [Int] = []

    private let columnTitle = NSTextField(labelWithString: "")
    private let columnType = noteLabel()
    private let statsGrid = NSStackView()
    private let chart = HistogramView()
    private let topValues = NSStackView()
    private let fileDetails = NSStackView()
    private let content = NSStackView()

    private static let cellID = NSUserInterfaceItemIdentifier("inspectorColumn")
    private static let topCount: UInt32 = 8

    override func loadView() {
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 280, height: 600))

        columnSearch.placeholderString = "Find column"
        columnSearch.delegate = self
        columnSearch.controlSize = .small

        let column = NSTableColumn(identifier: Self.cellID)
        columnList.addTableColumn(column)
        columnList.headerView = nil
        columnList.style = .plain
        columnList.rowHeight = 22
        columnList.backgroundColor = .clear
        columnList.dataSource = self
        columnList.delegate = self
        let listScroll = NSScrollView()
        listScroll.documentView = columnList
        listScroll.hasVerticalScroller = true
        listScroll.autohidesScrollers = true
        listScroll.drawsBackground = false
        listScroll.heightAnchor.constraint(equalToConstant: 150).isActive = true

        columnTitle.font = .systemFont(ofSize: 15, weight: .semibold)
        columnTitle.lineBreakMode = .byTruncatingTail
        let titleRow = NSStackView(views: [columnTitle, columnType])
        titleRow.orientation = .horizontal
        titleRow.alignment = .firstBaseline
        titleRow.spacing = 6

        for stack in [statsGrid, topValues, fileDetails] {
            stack.orientation = .vertical
            stack.alignment = .leading
            stack.spacing = 4
        }
        chart.isInteractive = false
        chart.heightAnchor.constraint(equalToConstant: 90).isActive = true

        content.orientation = .vertical
        content.alignment = .leading
        content.spacing = 8
        content.edgeInsets = NSEdgeInsets(top: 12, left: 14, bottom: 16, right: 14)
        for v in [
            Self.section("Columns"), columnSearch, listScroll, Self.divider(),
            titleRow, statsGrid, chart, topValues, Self.divider(),
            Self.section("File"), fileDetails,
        ] as [NSView] {
            content.addArrangedSubview(v)
            if v !== titleRow, v !== statsGrid, v !== topValues, v !== fileDetails {
                v.translatesAutoresizingMaskIntoConstraints = false
            }
        }
        for v in [columnSearch, listScroll, chart, statsGrid, topValues, fileDetails] as [NSView] {
            v.widthAnchor.constraint(equalTo: content.widthAnchor, constant: -28).isActive = true
        }
        content.setCustomSpacing(16, after: listScroll)
        content.setCustomSpacing(12, after: chart)

        // The whole inspector scrolls when it is taller than the window.
        let scroll = NSScrollView()
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        content.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(content)
        scroll.documentView = document
        scroll.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(scroll)
        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: root.safeAreaLayoutGuide.topAnchor),
            scroll.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            document.widthAnchor.constraint(equalTo: scroll.contentView.widthAnchor),
            content.topAnchor.constraint(equalTo: document.topAnchor),
            content.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            content.trailingAnchor.constraint(equalTo: document.trailingAnchor),
            content.bottomAnchor.constraint(equalTo: document.bottomAnchor),
        ])
        view = root
        showColumn()
    }

    // MARK: - Input from the window

    func show(doc: GewuDocument, info: FileInfo, kinds: [ColumnKind]) {
        self.doc = doc
        self.info = info
        self.kinds = kinds
        loadViewIfNeeded()
        filterColumns()
        showFile(info)
        if selected == nil, !info.columns.isEmpty { select(column: 0) }
    }

    func setFiltered(_ columns: Set<Int>) {
        filtered = columns
        columnList.reloadData()
    }

    /// Pick the column whose statistics to show.
    func select(column: Int) {
        selected = column
        if let row = shownColumns.firstIndex(of: column) {
            columnList.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
            columnList.scrollRowToVisible(row)
        } else {
            columnList.deselectAll(nil)
        }
        refresh()
    }

    /// The view changed (filters, search): recompute for its rows.
    func viewChanged(_ state: ViewState) {
        self.state = state
        refresh()
    }

    // MARK: - Column statistics

    private func refresh() {
        guard isViewLoaded else { return }
        showColumn()
        guard let doc, let column = selected else { return }
        generation += 1
        let mine = generation
        let state = state ?? ViewState(sort: [], filters: [], search: nil)
        let numeric = column < kinds.count && kinds[column] != .text
        let top = Self.topCount
        Task {
            let result = try? await Task.detached { () -> (ColumnSummary, Histogram?) in
                let summary = try doc.columnSummary(column: UInt32(column), top: top)
                let histogram = numeric
                    ? try? doc.histogram(column: UInt32(column), state: state, bins: 40)
                    : nil
                return (summary, histogram)
            }.value
            // A newer request replaced this one.
            guard mine == generation, let (summary, histogram) = result else { return }
            showStats(summary, histogram: histogram, column: column)
        }
    }

    private func showColumn() {
        guard let info, let column = selected, column < info.columns.count else {
            columnTitle.stringValue = "No column"
            columnType.stringValue = ""
            setRows(statsGrid, [])
            chart.isHidden = true
            topValues.isHidden = true
            return
        }
        let c = info.columns[column]
        columnTitle.stringValue = c.name
        columnType.stringValue = c.typeName + (c.nullable ? " · nullable" : "")
        columnType.toolTip = c.typeName
    }

    private func showStats(_ s: ColumnSummary, histogram: Histogram?, column: Int) {
        let kind = column < kinds.count ? kinds[column] : .text
        let percent = { (n: UInt64) -> String in
            s.rows == 0 ? "" : String(format: " (%.1f%%)", Double(n) / Double(s.rows) * 100)
        }
        var rows: [(String, String)] = [
            ("Rows in view", formatCount(s.rows)),
            ("Nulls", formatCount(s.nullCount) + percent(s.nullCount)),
        ]
        if s.nanCount > 0 { rows.append(("NaN", formatCount(s.nanCount) + percent(s.nanCount))) }
        rows.append(("Distinct", formatCount(s.distinctCount)))
        if let min = s.min, let max = s.max {
            rows.append(("Min", value(min, column: column, kind: kind)))
            rows.append(("Max", value(max, column: column, kind: kind)))
            if let mean = s.mean { rows.append(("Mean", value(mean, column: column, kind: kind))) }
            // A spread of dates in raw units would mean nothing here.
            if kind == .number, let sd = s.stdDev { rows.append(("Std dev", Self.decimal(sd))) }
        }
        setRows(statsGrid, rows)

        if let h = histogram, let lo = h.lo, let hi = h.hi {
            chart.labelFormatter = kind == .temporal
                ? { [weak self] v in self?.value(v, column: column, kind: kind) ?? "" }
                : nil
            chart.setData(counts: h.counts, lo: lo, hi: hi, integer: h.integer)
            chart.isHidden = false
        } else {
            chart.isHidden = true
        }

        topValues.arrangedSubviews.forEach { $0.removeFromSuperview() }
        topValues.isHidden = s.topValues.isEmpty
        if !s.topValues.isEmpty {
            topValues.addArrangedSubview(Self.section("Most common"))
            let most = s.topValues.first?.count ?? 1
            for v in s.topValues {
                topValues.addArrangedSubview(Self.valueRow(v.label ?? "", count: v.count, most: most))
            }
        }
    }

    /// A number, date or time for display.
    private func value(_ v: Double, column: Int, kind: ColumnKind) -> String {
        if kind == .temporal, let doc,
           let text = try? doc.formatValue(column: UInt32(column), value: Int64(v.rounded()))
        {
            return text
        }
        return Self.decimal(v)
    }

    /// Grouped digits with up to three decimals; scientific only when huge
    /// or tiny.
    private static func decimal(_ v: Double) -> String {
        let magnitude = abs(v)
        if magnitude != 0, magnitude >= 1e15 || magnitude < 1e-3 { return String(format: "%.4g", v) }
        let f = NumberFormatter()
        f.numberStyle = .decimal
        f.maximumFractionDigits = 3
        return f.string(from: NSNumber(value: v)) ?? String(v)
    }

    // MARK: - File

    private func showFile(_ info: FileInfo) {
        var rows: [(String, String)] = [
            ("Name", (info.path as NSString).lastPathComponent),
            ("Rows", formatCount(info.rowCount)),
            ("Size", ByteCountFormatter.string(fromByteCount: Int64(info.fileSize), countStyle: .file)),
            ("Row groups", formatCount(UInt64(info.rowGroups))),
            ("Compression", info.compression.map { $0.lowercased() }.joined(separator: ", ")),
        ]
        if let createdBy = info.createdBy { rows.append(("Created by", createdBy)) }
        for kv in info.metadata { rows.append((kv.key, kv.value)) }
        setRows(fileDetails, rows)
    }

    // MARK: - Column list

    private func filterColumns() {
        guard let info else { return }
        let text = columnSearch.stringValue.lowercased()
        shownColumns = info.columns.indices.filter {
            text.isEmpty || info.columns[$0].name.lowercased().contains(text)
        }
        columnList.reloadData()
        if let selected, let row = shownColumns.firstIndex(of: selected) {
            columnList.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        }
    }

    func controlTextDidChange(_ obj: Notification) {
        filterColumns()
    }

    func numberOfRows(in tableView: NSTableView) -> Int { shownColumns.count }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard let info else { return nil }
        let cell = (tableView.makeView(withIdentifier: Self.cellID, owner: self) as? ColumnListCell)
            ?? ColumnListCell(identifier: Self.cellID)
        let i = shownColumns[row]
        cell.name.stringValue = info.columns[i].name
        cell.type.stringValue = shortTypeName(info.columns[i].typeName)
        cell.funnel.isHidden = !filtered.contains(i)
        return cell
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        let row = columnList.selectedRow
        guard row >= 0, row < shownColumns.count, shownColumns[row] != selected else { return }
        selected = shownColumns[row]
        onSelectColumn?(shownColumns[row])
        refresh()
    }

    // MARK: - Building blocks

    private func setRows(_ stack: NSStackView, _ rows: [(String, String)]) {
        stack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        guard !rows.isEmpty else { return }
        let grid = NSGridView(views: rows.map { key, value in
            let k = noteLabel(key)
            k.font = .systemFont(ofSize: 12)
            let v = NSTextField(labelWithString: value)
            v.font = .monospacedDigitSystemFont(ofSize: 12, weight: .regular)
            v.lineBreakMode = .byTruncatingMiddle
            v.toolTip = value
            v.isSelectable = true
            v.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
            return [k, v]
        })
        grid.rowSpacing = 4
        grid.columnSpacing = 10
        grid.column(at: 0).xPlacement = .leading
        stack.addArrangedSubview(grid)
    }

    private static func section(_ title: String) -> NSTextField {
        let label = NSTextField(labelWithString: title)
        label.font = .systemFont(ofSize: 11, weight: .semibold)
        label.textColor = .secondaryLabelColor
        return label
    }

    private static func divider() -> NSBox {
        let box = NSBox()
        box.boxType = .separator
        return box
    }

    private static func valueRow(_ label: String, count: UInt64, most: UInt64) -> NSView {
        let name = NSTextField(labelWithString: label)
        name.font = .systemFont(ofSize: 12)
        name.lineBreakMode = .byTruncatingTail
        name.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        let bar = ProportionBar()
        bar.fraction = CGFloat(count) / CGFloat(max(most, 1))
        bar.widthAnchor.constraint(equalToConstant: 40).isActive = true
        bar.heightAnchor.constraint(equalToConstant: 10).isActive = true
        let n = noteLabel(formatCount(count))
        n.font = .monospacedDigitSystemFont(ofSize: 11, weight: .regular)
        n.alignment = .right
        let row = NSStackView(views: [name, NSView(), bar, n])
        row.orientation = .horizontal
        row.spacing = 6
        return row
    }
}

private final class FlippedView: NSView {
    override var isFlipped: Bool { true }
}

private final class ColumnListCell: NSTableCellView {
    let name = NSTextField(labelWithString: "")
    let type = NSTextField(labelWithString: "")
    let funnel = NSImageView()

    init(identifier: NSUserInterfaceItemIdentifier) {
        super.init(frame: .zero)
        self.identifier = identifier
        name.font = .systemFont(ofSize: 12)
        name.lineBreakMode = .byTruncatingTail
        name.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        type.font = .systemFont(ofSize: 11)
        type.textColor = .secondaryLabelColor
        type.setContentHuggingPriority(.required, for: .horizontal)
        funnel.image = filterSymbol
        funnel.setContentHuggingPriority(.required, for: .horizontal)
        let stack = NSStackView(views: [name, funnel, NSView(), type])
        stack.orientation = .horizontal
        stack.spacing = 6
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        textField = name
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 4),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            stack.centerYAnchor.constraint(equalTo: centerYAnchor),
            funnel.widthAnchor.constraint(equalToConstant: 12),
            funnel.heightAnchor.constraint(equalToConstant: 12),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }
}

