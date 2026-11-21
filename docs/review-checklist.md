# Review checklist

The rules a change must not break, grouped by the code it touches. Each rule names what guards it: a compiler or CI check, a test (by name, so you can find it), or **review** when nothing automated does. Rules guarded only by review deserve the closest look. The [architecture](architecture.md) explains the model behind these rules, and the [decision records](decisions/README.md) explain why.

Beyond correctness, review for one clear model: consistent names across layers, no concept left behind by a refactor, and documentation that still matches the code.

## Any change

| Rule | Guarded by |
| --- | --- |
| Dependencies point one way: App → GewuKit → FFI → Core. | Crate and target dependencies; review |
| Generated bindings, C headers, module maps and `Gewu.xcodeproj` are never edited by hand. Edit the Rust interface or `project.yml`. | Regenerated on every build ([build-rust.sh](../scripts/build-rust.sh)); review |
| A behavior is tested once, at the stable boundary that owns it, not repeated across Core, FFI and Swift. | Review |
| A change that moves a layer boundary or makes a new decision updates the [architecture](architecture.md) or adds a [decision record](decisions/README.md). | Review |

## Core

| Rule | Guarded by |
| --- | --- |
| Core has no UniFFI (or other binding) dependency, derive or attribute. | Review: `crates/gewu-core/Cargo.toml` |
| No frontend concepts: windows, visible columns, selection, panels, current view, wording. | Review |
| Views are immutable. Producing a new view never changes an existing one, and Find and export capture what they need. | `analysis_and_scatter_remain_bound_to_independent_views`; review |
| Filters run against the whole dataset, never against the previous view. | `dataset_queries_recompute_from_the_full_source` |
| A column's own filter never hides its alternatives in its editor (facet). | `choices_ignore_only_the_edited_columns_filter`, `histogram_domain_and_counts_use_rows_passing_the_other_filters` |
| Nulls sort last in both directions; equal keys keep source order. | `sorts_with_nulls_last`, `kernels_preserve_arrow_native_and_multikey_order` |
| Exact values and range bounds round-trip losslessly as text; labels and plot coordinates are never filter input. Dictionary codes never leave Core. | `candidate_values_round_trip_large_integers_decimals_and_zoned_timestamps`, `provided_bin_bounds_round_trip_above_double_integer_precision`, `exact_values_and_captured_facets_keep_byte_and_null_semantics` |
| Query execution reads native Arrow arrays; it does not format or widen whole columns. | Review; benchmarks (`cargo bench -p gewu-core --bench engine`) |
| In-memory scans have no cancellation checks ([ADR 0009](decisions/0009-no-in-memory-cancellation.md)). | Review |
| Export never replaces the destination unless it completes, and refuses to overwrite its source. | `cancellation_and_failures_preserve_existing_destination_and_remove_temporary_files`, `source_file_and_aliases_cannot_be_overwritten` |
| Errors belong to the module that owns the failed operation and keep their source. | Review |
| Each codec feature builds and passes alone. | Manual: `cargo test -p gewu-core --no-default-features --features <f>` ([development guide](../DEVELOPMENT.md#test-and-format)); not in CI |

## FFI and the data plane

| Rule | Guarded by |
| --- | --- |
| No query algorithms or codecs; those stay in Core. No interaction state; no scheduling (every export is synchronous). | Review |
| Core types cross with `#[uniffi::remote]` when their shape suits the boundary; own types only where the boundary needs another shape. | Review |
| `apply` returns the same view when nothing changed and keeps the filtered view when only the sort changed. Swift follows `is_same_view`. | `applying_a_query_reuses_what_did_not_change` (FFI), `applyReusesWhatTheQueryDidNotChange` (GewuKit), `sortChangesKeepTheRowSetAndItsAnalyses` (App) |
| The analysis cache keeps one result per analysis and column, never caches failures, and stays bounded by the schema. | `a_slot_keeps_one_result_and_recomputes_for_new_parameters` |
| A lent pointer points into memory the UniFFI object owns and never mutates. Each lending function consumes exactly one cloned handle. | `source_rows_lend_the_views_own_indices`, `ordered_views_lend_their_source_rows`; review of every `unsafe` block and its `SAFETY` comment |
| A Swift reader of lent memory keeps the owning object alive for as long as it reads (`RowNumbers.owner`, `withExtendedLifetime`). | `sourceRowsAreReadInPlaceForEveryOrder`; review |
| `ColumnText` layout (offsets, validity, text) matches on both sides. | `column_text_packs_offsets_validity_and_text`, `columnTextDecodesCellsInPlace` |
| New bulk data uses one of the two routes in [ADR 0008](decisions/0008-data-plane.md): packed value, or typed pointer through `gewu_data.h`. No pointers passed as integers through UniFFI. | Review |
| Errors keep structured kinds; only contract violations become `Internal`. A filter failure keeps its `Filter` kind wherever it arises. | `errors_keep_their_structured_kinds`, `filter_failures_inside_analyses_keep_their_filter_kind` |
| Interface changes are validated on both sides: Rust tests and GewuKit tests through the generated bindings. | CI (Rust and App workflows); review |

## GewuKit

| Rule | Guarded by |
| --- | --- |
| Only GewuKit imports `GewuBindings` or `gewuFFI`, with `internal import`. No binding type appears in its public API. | CI step "Keep the bindings private to GewuKit"; compiler |
| Result types (`TableSource`, `RowSet`, `DisplayRows`) are immutable and compare by identity. | Review |
| Query equality is exact: filter literals compare by UTF-8 bytes, and filters keep column order, so equal queries select the same rows. | `literalsCompareByBytesAsTheEngineDoes`, `canonicallyEquivalentValuesAreDifferentQueries`, `filtersKeepOneOrderWhateverOrderTheyWereSetIn`, `exactValuesCompareBytesAndNullsAreIndependent` |
| Every Rust call runs on a serial worker, never on the main actor or the cooperative pool. A call cancelled before it starts never enters Rust. | `queuedWorkCancelledBeforeItStartsNeverRuns`; review |
| Cancellation, from Swift or Rust, surfaces as `CancellationError`, never as a `GewuKitError`. | `rustCancellationArrivesAsCancellationError` |
| `GewuKitError` carries a kind and the English diagnostic, never user-facing wording. Translation from the bindings is exhaustive. | Compiler (exhaustive switches); review |
| Edit rules live in `TableState` and `FilterDraft`: hiding a column clears its conditions, choices fall back to visible columns, sorts keep priority. | `hidingAColumnClearsItsConditionsAndMovesChoicesToVisibleColumns`, `sortsCycleAndKeepPriorityWhateverTheColumnOrder`, `eachModeKeepsItsDraftAndReopensFromItsFilter` |
| One waiter cancelling does not cancel another waiter's analysis. | `oneWaiterCancellingDoesNotCancelAnother` |

## App

| Rule | Guarded by |
| --- | --- |
| No imports of the bindings and no FFI type names. | CI; compiler |
| Swift 6 language mode; no `@unchecked Sendable` in the App. | Compiler (`SWIFT_VERSION: '6.0'` in [project.yml](../project.yml)); review |
| Each piece of state has one owner whose lifetime matches it; models are `@Observable` and never refer to views. | Review |
| `displayed` changes only when a query succeeds. A failed query keeps it and returns the draft to it. | `failedQueriesKeepTheDisplayedRowsAndRestoreTheDraft`, `queryEditsPublishOnlyTheirFinalDraft` |
| A cancelled task changes no state when it ends, whether it succeeded or failed. | `revertingAPendingFilterCancelsItsQuery`, `lateReadsForReplacedRowsAreDiscarded`; review of every task handler |
| The table never shows a blank, partial or mixed frame while rows change. | `newRowsWaitUntilEveryVisibleValueIsReady`, `aSupersededSwitchNeverReplacesTheNewerRows`, `testFilterPublishesWholeTableFrames` (`GewuPerformance` scheme, run manually) |
| Column visibility and order are presentation: they apply at once and never run a query. | `showingAndHidingColumnsKeepsRowsAndLoadedText`, `hidingAColumnAppliesAtOnceAndClearsItsConditions` |
| Panels load in `.task(id:)` keyed by the result identity, column and parameters. | Review |
| `ErrorPresentation` is the only place that words engine failures. | `engineFailuresAreWordedFromTheirKind`; review |
| Copy is exactly a TSV export of the selected rows and visible columns. | `copyPutsTheLatestSelectionOnThePasteboardAsTSV`, `tabSeparatedTextQuotesFieldsAndRespectsItsLimit` |

## Reporting what was checked

State which of these ran: build, automated tests (which suites), UI verification in the running app, performance measurement. They are different claims.
