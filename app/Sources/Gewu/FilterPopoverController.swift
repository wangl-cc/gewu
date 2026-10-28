import AppKit

/// Filter for text (and other non-numeric) columns.
///
/// With few distinct values it is an Excel-style checkbox list. With many,
/// checking boxes one by one is useless, so it becomes a "contains" search
/// whose list previews the matching values.
final class FilterPopoverController: NSViewController, NSTableViewDataSource, NSTableViewDelegate {
    /// Called with the column's new filters; empty means no filter.
    private let onApply: ([Filter]) -> Void
    private let doc: GewuDocument
    private let column: UInt32
    private let state: ViewState
    private let initialKeys: Set<UInt64>?
    private let initialText: String
    private var nullMode: NullMode

    private var values: [DistinctValue] = []
    private var shown: [Int] = []
    private var checked: Set<UInt64> = []
    private var truncated = false
    private var maxCount: UInt64 = 0
    /// Contains mode: too many values for a checkbox list.
    private var containsMode = false

    private let list = NSTableView()
    private let searchField = NSSearchField()
    private let note = noteLabel("Loading…")
    private lazy var selectAll = NSButton(title: "Select All", target: self, action: #selector(checkAllShown(_:)))
    private lazy var clear = NSButton(title: "Clear", target: self, action: #selector(uncheckAllShown(_:)))
    private lazy var nulls = NullMode.popUp(nullCount: nil, target: self, action: #selector(nullModeChanged(_:)))

    /// Above this many distinct values, use contains mode.
    private static let checkboxLimit: UInt64 = 500
    private static let fetchLimit: UInt32 = 10_000
    private static let cellID = NSUserInterfaceItemIdentifier("check")

    init(doc: GewuDocument, column: UInt32, state: ViewState, current: [Filter],
         onApply: @escaping ([Filter]) -> Void)
    {
        self.doc = doc
        self.column = column
        self.state = state
        self.onApply = onApply
        var keys: Set<UInt64>?
        var text = ""
        for filter in current {
            switch filter {
            case let .inValues(_, k): keys = Set(k)
            case let .contains(_, t, _): text = t
            default: break
            }
        }
        initialKeys = keys
        initialText = text
        nullMode = NullMode(current)
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    override func loadView() {
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 360, height: 420))

        searchField.placeholderString = "Find value"
        searchField.target = self
        searchField.action = #selector(searchChanged(_:))

        let col = NSTableColumn(identifier: Self.cellID)
        col.width = 330
        list.addTableColumn(col)
        list.headerView = nil
        list.rowHeight = 22
        list.dataSource = self
        list.delegate = self

        let scroll = NSScrollView()
        scroll.documentView = list
        scroll.hasVerticalScroller = true
        scroll.borderType = .bezelBorder

        nulls.isHidden = true
        let ok = NSButton(title: "OK", target: self, action: #selector(apply(_:)))
        ok.keyEquivalent = "\r"
        let buttons = NSStackView(views: [selectAll, clear, nulls, NSView(), ok])
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
        let doc = doc, column = column, state = state, limit = Self.fetchLimit
        Task {
            do {
                let result = try await Task.detached {
                    try doc.distinctValues(column: column, state: state, limit: limit)
                }.value
                truncated = result.truncated
                containsMode = result.total > Self.checkboxLimit
                values = result.values
                if containsMode {
                    // Most common first, as a guide to what to type.
                    values.sort { $0.count > $1.count }
                }
                maxCount = values.map(\.count).max() ?? 0
                checked = initialKeys ?? Set(values.map(\.key))
                configureMode(total: result.total)
                applySearch()
            } catch {
                note.stringValue = error.localizedDescription
            }
        }
    }

    private func configureMode(total: UInt64) {
        if containsMode {
            searchField.placeholderString = "Contains…"
            searchField.stringValue = initialText
            selectAll.isHidden = true
            clear.isHidden = true
            nulls.isHidden = false
            nulls.selectItem(at: nullMode.rawValue)
            view.window?.makeFirstResponder(searchField)
        }
        updateNote(total: total)
    }

    private func updateNote(total: UInt64? = nil) {
        if containsMode {
            let scope = truncated ? "the first \(formatCount(UInt64(values.count)))" : "all"
            note.stringValue = searchField.stringValue.isEmpty
                ? "\(formatCount(total ?? UInt64(values.count))) distinct values · type to filter by text"
                : "\(formatCount(UInt64(shown.count))) matching values (searched \(scope))"
        } else {
            note.stringValue = "\(formatCount(UInt64(values.count))) values"
        }
    }

    // MARK: - List

    func numberOfRows(in tableView: NSTableView) -> Int { shown.count }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let cell = (tableView.makeView(withIdentifier: Self.cellID, owner: self) as? FilterValueCell)
            ?? FilterValueCell(identifier: Self.cellID, target: self, action: #selector(toggle(_:)))
        let value = values[shown[row]]
        cell.checkbox.title = value.label ?? "(null)"
        // In contains mode the list is a preview: no checkboxes.
        cell.checkbox.imagePosition = containsMode ? .noImage : .imageLeading
        cell.checkbox.state = checked.contains(value.key) ? .on : .off
        cell.checkbox.tag = row
        cell.count.stringValue = formatCount(value.count)
        cell.bar.fraction = maxCount > 0 ? CGFloat(value.count) / CGFloat(maxCount) : 0
        return cell
    }

    @objc private func toggle(_ sender: NSButton) {
        guard sender.tag < shown.count else { return }
        let value = values[shown[sender.tag]]
        if containsMode {
            // Clicking a value fills the search with it.
            searchField.stringValue = value.label ?? ""
            applySearch()
            return
        }
        if sender.state == .on { checked.insert(value.key) } else { checked.remove(value.key) }
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
        updateNote()
    }

    @objc private func checkAllShown(_ sender: Any?) {
        shown.forEach { checked.insert(values[$0].key) }
        list.reloadData()
    }

    @objc private func uncheckAllShown(_ sender: Any?) {
        shown.forEach { checked.remove(values[$0].key) }
        list.reloadData()
    }

    @objc private func nullModeChanged(_ sender: NSPopUpButton) {
        nullMode = NullMode(rawValue: sender.indexOfSelectedItem) ?? .include
    }

    @objc private func apply(_ sender: Any?) {
        if containsMode {
            if let only = nullMode == .only ? nullMode.filter(column: column) : nil {
                onApply([only])
                return
            }
            var filters: [Filter] = []
            let text = searchField.stringValue
            if !text.isEmpty {
                filters.append(.contains(column: column, text: text, caseSensitive: false))
            }
            // "contains" already drops nulls; only add the null filter alone.
            if text.isEmpty, let f = nullMode.filter(column: column) { filters.append(f) }
            onApply(filters)
        } else {
            let all = Set(values.map(\.key))
            let everything = !truncated && all.isSubset(of: checked)
            onApply(everything ? [] : [.inValues(column: column, keys: checked.sorted())])
        }
    }
}

/// One value in the filter list: checkbox, a bar for its share, and its count.
private final class FilterValueCell: NSTableCellView {
    let checkbox: NSButton
    let bar = ProportionBar()
    let count = NSTextField(labelWithString: "")

    init(identifier: NSUserInterfaceItemIdentifier, target: AnyObject, action: Selector) {
        checkbox = NSButton(checkboxWithTitle: "", target: target, action: action)
        super.init(frame: .zero)
        self.identifier = identifier
        checkbox.lineBreakMode = .byTruncatingTail
        checkbox.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        count.font = .monospacedDigitSystemFont(ofSize: 11, weight: .regular)
        count.textColor = .secondaryLabelColor
        count.alignment = .right

        for v in [checkbox, bar, count] as [NSView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            addSubview(v)
        }
        NSLayoutConstraint.activate([
            checkbox.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 2),
            checkbox.centerYAnchor.constraint(equalTo: centerYAnchor),
            checkbox.trailingAnchor.constraint(lessThanOrEqualTo: bar.leadingAnchor, constant: -8),

            bar.widthAnchor.constraint(equalToConstant: 48),
            bar.heightAnchor.constraint(equalToConstant: 10),
            bar.centerYAnchor.constraint(equalTo: centerYAnchor),
            bar.trailingAnchor.constraint(equalTo: count.leadingAnchor, constant: -8),

            count.widthAnchor.constraint(equalToConstant: 64),
            count.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -4),
            count.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }
}
