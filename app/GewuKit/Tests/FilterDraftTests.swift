import GewuKit
import Testing

@Suite struct FilterDraftTests {
    @Test func exactValuesCompareBytesAndNullsAreIndependent() {
        let composed = ExactValue("é")
        let decomposed = ExactValue("e\u{301}")
        #expect(composed != decomposed)
        var selection = ExactSelection(nil)
        selection.set(composed, selected: false)
        #expect(!selection.contains(composed) && selection.contains(decomposed))
        let filter = NullMode.exclude.applying(to: selection.condition)
        #expect(filter == .condition(.exact(values: ["é"], exclude: true), includeNulls: false))
        #expect(NullMode.only.applying(to: selection.condition) == .onlyNulls)
        selection.setAll(true)
        #expect(NullMode.include.applying(to: selection.condition) == nil, "Excluding nothing is no filter")
        #expect(NullMode.exclude.applying(to: selection.condition) == .notNull)
    }

    @Test func eachModeKeepsItsDraftAndReopensFromItsFilter() {
        let range = Filter.condition(.range(min: "1", max: nil), includeNulls: false)
        var draft = FilterDraft(range)
        #expect(draft.nullMode == .exclude && draft.range.isTyped)
        draft.text.text = "abc"
        #expect(draft.filter(mode: .range) == range, "Editing another mode leaves the range")
        #expect(
            draft.filter(mode: .text)
                == .condition(.text(text: "abc", mode: .contains, caseSensitive: false), includeNulls: false))
        #expect(draft.filter(mode: .range, hasContent: false) == .notNull)

        draft.nullMode = .only
        draft.setValue(ExactValue("v"), selected: true)
        #expect(draft.nullMode == .include, "Choosing a value leaves the null-only mode")
        #expect(draft.filter(mode: .exact) == .condition(.exact(values: ["v"], exclude: false), includeNulls: true))
        draft.reset()
        #expect(draft.filter(mode: .exact) == nil && draft.filter(mode: .range) == nil)
    }
}
