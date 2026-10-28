import AppKit

/// Excel-style filter: a checkbox list of a column's distinct values.
final class FilterPopoverController: NSViewController, NSTableViewDataSource, NSTableViewDelegate {
    /// Called with the chosen keys, or `nil` when every value is chosen
    /// (meaning: no filter on this column).
    private let onApply: (Set<UInt64>?) -> Void
    private let doc: GewuDocument
    private let column: UInt32
    private let state: ViewState
    private let initial: Set<UInt64>?

    private var values: [DistinctValue] = []
    private var shown: [Int] = []
    private var checked: Set<UInt64> = []
    private var truncated = false

    private let list = NSTableView()
    private let searchField = NSSearchField()
    private let note = NSTextField(labelWithString: "Loading…")

    private static let limit: UInt32 = 10_000
    private static let cellID = NSUserInterfaceItemIdentifier("check")

    init(doc: GewuDocument, column: UInt32, state: ViewState, selected: Set<UInt64>?,
         onApply: @escaping (Set<UInt64>?) -> Void)
    {
        self.doc = doc
        self.column = column
        self.state = state
        self.initial = selected
        self.onApply = onApply
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    override func loadView() {
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 300, height: 380))

        searchField.placeholderString = "Find value"
        searchField.target = self
        searchField.action = #selector(searchChanged(_:))

        let col = NSTableColumn(identifier: Self.cellID)
        col.width = 270
        list.addTableColumn(col)
        list.headerView = nil
        list.rowHeight = 20
        list.dataSource = self
        list.delegate = self

        let scroll = NSScrollView()
        scroll.documentView = list
        scroll.hasVerticalScroller = true
        scroll.borderType = .bezelBorder

        note.textColor = .secondaryLabelColor
        note.font = .systemFont(ofSize: NSFont.smallSystemFontSize)

        let selectAll = NSButton(title: "Select All", target: self, action: #selector(selectAll(_:)))
        let clear = NSButton(title: "Clear", target: self, action: #selector(clearAll(_:)))
        let ok = NSButton(title: "OK", target: self, action: #selector(apply(_:)))
        ok.keyEquivalent = "\r"
        let buttons = NSStackView(views: [selectAll, clear, NSView(), ok])
        buttons.orientation = .horizontal

        for v in [searchField, scroll, note, buttons] as [NSView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            root.addSubview(v)
        }
        NSLayoutConstraint.activate([
            searchField.topAnchor.constraint(equalTo: root.topAnchor, constant: 10),
            searchField.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 10),
            searchField.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -10),

            scroll.topAnchor.constraint(equalTo: searchField.bottomAnchor, constant: 8),
            scroll.leadingAnchor.constraint(equalTo: searchField.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: searchField.trailingAnchor),

            note.topAnchor.constraint(equalTo: scroll.bottomAnchor, constant: 4),
            note.leadingAnchor.constraint(equalTo: searchField.leadingAnchor),
            note.trailingAnchor.constraint(equalTo: searchField.trailingAnchor),

            buttons.topAnchor.constraint(equalTo: note.bottomAnchor, constant: 6),
            buttons.leadingAnchor.constraint(equalTo: searchField.leadingAnchor),
            buttons.trailingAnchor.constraint(equalTo: searchField.trailingAnchor),
            buttons.bottomAnchor.constraint(equalTo: root.bottomAnchor, constant: -10),
        ])
        view = root
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        let doc = doc, column = column, state = state
        Task {
            do {
                let result = try await Task.detached {
                    try doc.distinctValues(column: column, state: state, limit: Self.limit)
                }.value
                values = result.values
                truncated = result.truncated
                checked = initial ?? Set(values.map(\.key))
                applySearch()
                note.stringValue = truncated
                    ? "Showing the first \(values.count) of \(result.total) values"
                    : "\(values.count) values"
            } catch {
                note.stringValue = error.localizedDescription
            }
        }
    }

    // MARK: - List

    func numberOfRows(in tableView: NSTableView) -> Int { shown.count }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let button = (tableView.makeView(withIdentifier: Self.cellID, owner: self) as? NSButton)
            ?? {
                let b = NSButton(checkboxWithTitle: "", target: self, action: #selector(toggle(_:)))
                b.identifier = Self.cellID
                b.lineBreakMode = .byTruncatingTail
                return b
            }()
        let value = values[shown[row]]
        let label = value.label ?? "(null)"
        button.title = "\(label)  (\(value.count))"
        button.state = checked.contains(value.key) ? .on : .off
        button.tag = row
        return button
    }

    @objc private func toggle(_ sender: NSButton) {
        guard sender.tag < shown.count else { return }
        let key = values[shown[sender.tag]].key
        if sender.state == .on { checked.insert(key) } else { checked.remove(key) }
    }

    @objc private func searchChanged(_ sender: NSSearchField) {
        applySearch()
    }

    private func applySearch() {
        let text = searchField.stringValue.lowercased()
        shown = values.indices.filter { i in
            text.isEmpty || (values[i].label ?? "(null)").lowercased().contains(text)
        }
        list.reloadData()
    }

    @objc private func selectAll(_ sender: Any?) {
        shown.forEach { checked.insert(values[$0].key) }
        list.reloadData()
    }

    @objc private func clearAll(_ sender: Any?) {
        shown.forEach { checked.remove(values[$0].key) }
        list.reloadData()
    }

    @objc private func apply(_ sender: Any?) {
        let all = Set(values.map(\.key))
        let everything = !truncated && all.isSubset(of: checked)
        onApply(everything ? nil : checked)
    }
}
