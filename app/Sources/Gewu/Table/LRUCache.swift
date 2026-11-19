/// A least-recently-used map bounded by the total cost of its entries.
/// A dictionary indexes the nodes of a doubly linked list stored in an array,
/// so lookups, insertions and evictions are O(1). Freed slots are reused.
final class LRUCache<Key: Hashable, Value> {
    private struct Node {
        var key: Key
        var value: Value?
        var cost: Int
        var newer: Int
        var older: Int
    }

    /// Total cost the cache retains.
    let costLimit: Int
    /// An entry costing more than this is never retained, so one large value
    /// cannot evict the rest of the cache.
    let entryCostLimit: Int
    private(set) var totalCost = 0
    private var nodes: [Node] = []
    private var freeSlots: [Int] = []
    private var slots: [Key: Int] = [:]
    private var newest = -1
    private var oldest = -1

    init(costLimit: Int, entryCostLimit: Int) {
        precondition(costLimit >= 0 && entryCostLimit >= 0)
        self.costLimit = costLimit
        self.entryCostLimit = min(entryCostLimit, costLimit)
    }

    var count: Int { slots.count }
    var onRemove: ((Key, Value) -> Void)?

    func contains(_ key: Key) -> Bool { slots[key] != nil }

    /// Return the value and mark it most recently used.
    func value(for key: Key) -> Value? {
        guard let slot = slots[key] else { return nil }
        if slot != newest {
            unlink(slot)
            linkNewest(slot)
        }
        return nodes[slot].value
    }

    /// Retain `value` unless its cost exceeds `entryCostLimit`. An existing
    /// value for `key` is replaced either way.
    @discardableResult
    func insert(_ value: Value, for key: Key, cost: Int) -> Bool {
        precondition(cost >= 0)
        removeValue(for: key)
        guard cost <= entryCostLimit else { return false }
        while totalCost + cost > costLimit, oldest >= 0 { remove(slot: oldest) }
        let node = Node(key: key, value: value, cost: cost, newer: -1, older: -1)
        let slot: Int
        if let free = freeSlots.popLast() {
            nodes[free] = node
            slot = free
        } else {
            nodes.append(node)
            slot = nodes.count - 1
        }
        slots[key] = slot
        totalCost += cost
        linkNewest(slot)
        return true
    }

    func removeValue(for key: Key) {
        if let slot = slots[key] { remove(slot: slot) }
    }

    /// O(n): intended for rare bulk invalidation.
    func removeAll(where shouldRemove: (Key) -> Bool) {
        for (key, slot) in slots where shouldRemove(key) { remove(slot: slot) }
    }

    func removeAll() {
        nodes.removeAll()
        freeSlots.removeAll()
        slots.removeAll()
        newest = -1
        oldest = -1
        totalCost = 0
    }

    private func remove(slot: Int) {
        unlink(slot)
        slots[nodes[slot].key] = nil
        totalCost -= nodes[slot].cost
        if let value = nodes[slot].value { onRemove?(nodes[slot].key, value) }
        nodes[slot].value = nil
        nodes[slot].cost = 0
        freeSlots.append(slot)
    }

    private func unlink(_ slot: Int) {
        let (newer, older) = (nodes[slot].newer, nodes[slot].older)
        if newer >= 0 { nodes[newer].older = older } else { newest = older }
        if older >= 0 { nodes[older].newer = newer } else { oldest = newer }
    }

    private func linkNewest(_ slot: Int) {
        nodes[slot].newer = -1
        nodes[slot].older = newest
        if newest >= 0 { nodes[newest].newer = slot }
        newest = slot
        if oldest < 0 { oldest = slot }
    }
}
