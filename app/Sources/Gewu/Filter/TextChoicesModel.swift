import SwiftUI

/// Retains one immutable View/column scope and one candidate page.
@MainActor
final class TextChoicesModel: ObservableObject {
    @Published private(set) var page: TextChoicePage?
    @Published private(set) var loading = false
    @Published private(set) var error: String?
    private var choices: TextChoices?
    private var context: Context?
    private struct Context: Equatable {
        let view: GewuView
        let column: UInt32
        static func == (lhs: Self, rhs: Self) -> Bool {
            lhs.view === rhs.view && lhs.column == rhs.column
        }
    }
    private var current: Cancellation?

    func load(
        view: GewuView, column: UInt32,
        query: String, after: TextChoiceCursor?
    ) async {
        guard !Task.isCancelled else { return }
        current?.cancel()
        let cancellation = Cancellation()
        current = cancellation
        loading = true
        error = nil
        page = nil
        defer { if current === cancellation { loading = false } }
        do {
            try await Task.sleep(for: .milliseconds(200))
            guard !Task.isCancelled, current === cancellation else { return }
            let next = Context(view: view, column: column)
            if context != next {
                choices?.cancel()
                choices = try view.textChoices(column: column)
                context = next
            }
            guard let choices else { return }
            let result = try await runCancellable(cancellation: cancellation) {
                try choices.page(query: query, after: after, limit: 100, cancellation: cancellation)
            }
            guard !Task.isCancelled, current === cancellation else { return }
            page = result
        } catch {
            guard !Task.isCancelled, current === cancellation else { return }
            if case GewuError.Cancelled = error { return }
            self.error = error.userMessage
        }
    }

    func pause() {
        current?.cancel()
        current = nil
        loading = false
        page = nil
        error = nil
    }

    func cancel() {
        current?.cancel()
        current = nil
        choices?.cancel()
        choices = nil
        context = nil
        loading = false
    }
}
