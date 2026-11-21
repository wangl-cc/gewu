# Development guide

How to change the code without breaking its model. Read the [architecture](architecture.md) first: this guide assumes its vocabulary (the three result objects and the four layers). The architecture explains why the rules below exist; this guide says how to keep them. Build, test and formatting commands are in the [development setup](../../DEVELOPMENT.md).

## General rules

- **One clear model.** A change leaves one way of doing each thing. When a concept is replaced, remove the old one everywhere: types, names, comments, tests and documentation. A name means the same thing in every layer; when you rename, rename across Core, FFI, GewuKit and the App.
- **Each fact in one place.** Architecture and rules live in `docs/dev/`, product behavior in [`docs/spec/`](../spec), layer-specific API contracts in that layer's README. Link instead of restating.
- **Documents change with the code.** In the same change, update the architecture when a layer's responsibilities, its forbidden contents, the result objects or the data plane change; update this guide when a rule, its reason or its guard changes; update the spec when product behavior changes.
- **New trade-offs are written down where the rule lives.** A change that picks one design over a reasonable alternative, or reverses an earlier one, updates the [architecture](architecture.md) or this guide in the same change, with the rejected alternative in a sentence. A large design that needs room for options gets a [design document](design/README.md).
- **Issues are for discussion, not record.** When a discussion in an issue or pull request settles something, its conclusion goes into this guide, the architecture or a design document; the docs never point to a thread as the source of a rule.
- **Test behavior once, at the boundary that owns it.** Do not repeat the same matrix in Core, FFI and Swift. A regression test must fail without the fix.
- **Generated files are outputs.** Edit the Rust interface, not the generated Swift bindings, C header or module map; edit `project.yml`, not `Gewu.xcodeproj`.
- **Report what was verified.** Build success, automated tests (which suites), UI checked in the running app and performance measurements are different claims; say which ran and which did not.

## Common changes

### Add or change an FFI method

1. Implement the behavior in Core if it is a data question; in the FFI only if it is a reuse, caching or boundary rule ([architecture: layers](architecture.md#layers)).
2. Export it from the owning object in `crates/gewu-uniffi/src`. New Core types that suit the boundary are declared there with `#[uniffi::remote]`; give the FFI its own type only when the boundary needs another shape ([architecture: Core](architecture.md#core-general-data-semantics)).
3. Map any new Core failure to a structured kind (see [Add a user-facing failure](#add-a-user-facing-failure)).
4. Test the Rust side in `crates/gewu-uniffi/src/tests.rs` when the FFI owns the behavior.
5. Wrap it in GewuKit: a method on the result type it belongs to, running on the right session worker through `SerialWorker.perform`, returning Swift values. Test it in `app/GewuKit/Tests` through the generated bindings.
6. Update the [FFI README](../../crates/gewu-uniffi/README.md) contract. The App changes only if GewuKit's public API changes.

### Add an analysis

1. Core: the kernel and its result record under `stats/`, its capability in `AnalysisCapabilities`, and a method on `FilteredView` in `view/analysis.rs`. It must not depend on row order.
2. FFI: a method on `GewuFilteredView` that goes through a slot in `cache.rs` (`Analyses`), keyed by column and parameters, and remote declarations for the result types.
3. GewuKit: a value type in `Model/Analysis.swift` and a method on `RowSet` that runs on the analysis worker.
4. App: present it in the Inspector, loading in `.task(id:)` keyed by the `RowSet`, column and parameters.
5. Document the semantics in the Core README and, if users see new behavior, in the spec.

### Add a user-facing failure

1. Core: a variant in the error type of the module that owns the operation, aggregated by `Error`.
2. FFI: a structured kind with the values needed to describe it, in `error.rs`. Failures a correct UI cannot cause become `Internal` instead.
3. GewuKit: the matching case in `GewuKitError.Kind` and its translation. The switches are exhaustive, so the compiler points to each.
4. App: the wording in `ErrorPresentation`, and whether the diagnostic is needed as details.

### Change filter, Find or value semantics

1. Change Core (`query/`, `column/predicate/`, `scalar/`, `find.rs`) and its tests.
2. Update the [spec](../spec) first-hand: it is the contract the change must satisfy.
3. Exact values and range bounds cross as lossless text; never send labels, plot coordinates or dictionary codes as filter input ([architecture: query path](architecture.md#the-query-path)).

### Add or change a file format

1. Core `file/`, behind its Cargo feature. Disabled formats must still compile and return `FormatDisabled`.
2. Test Core with no default features and with the feature alone.
3. Update [file formats](../spec/file-formats.md) and the format table in the README.

## Rules by layer

Each rule names its reason and what guards it: a compiler or CI check, a test by name, or **review** when nothing automated does. Rules guarded only by review deserve the closest look.

### Core

| Rule | Why | Guarded by |
| --- | --- | --- |
| No UniFFI (or other binding) dependency, derive or attribute. | Keeps the engine general ([architecture: Core](architecture.md#core-general-data-semantics)). | Review: `crates/gewu-core/Cargo.toml` |
| No frontend concepts: windows, visible columns, selection, panels, current view, wording. | Core serves data semantics, not one UI ([architecture: layers](architecture.md#layers)). | Review |
| Views are immutable; Find and export capture what they need. | Consistency without versions ([architecture: mental model](architecture.md#mental-model)). | `analysis_and_scatter_remain_bound_to_independent_views`; review |
| Filters run against the whole dataset, never against the previous view. | Relaxing a filter must bring rows back. | `dataset_queries_recompute_from_the_full_source` |
| A column's own filter never hides its alternatives in its editor. | Spreadsheet-style value filters. | `choices_ignore_only_the_edited_columns_filter`, `histogram_domain_and_counts_use_rows_passing_the_other_filters` |
| Nulls sort last in both directions; equal keys keep source order. | Stable, predictable order. | `sorts_with_nulls_last`, `kernels_preserve_arrow_native_and_multikey_order` |
| Exact values and range bounds round-trip losslessly as text; dictionary codes never leave Core. | [architecture: query path](architecture.md#the-query-path). | `candidate_values_round_trip_large_integers_decimals_and_zoned_timestamps`, `provided_bin_bounds_round_trip_above_double_integer_precision`, `exact_values_and_captured_facets_keep_byte_and_null_semantics` |
| Queries read native Arrow arrays; they do not format or widen whole columns. | Speed on large tables. | Review; engine benchmarks |
| In-memory scans have no cancellation checks. | [architecture: cancellation](architecture.md#concurrency-and-cancellation). | Review |
| Export never replaces the destination unless it completes, and never overwrites its source. | A failed or cancelled export must not lose data. | `cancellation_and_failures_preserve_existing_destination_and_remove_temporary_files`, `source_file_and_aliases_cannot_be_overwritten` |
| Errors belong to the module that owns the failed operation and keep their source. | Structured kinds downstream ([architecture: errors](architecture.md#errors)). | Review |
| Each codec feature builds and passes alone. | Features are additive. | Manual; not in CI |

### FFI and the data plane

| Rule | Why | Guarded by |
| --- | --- | --- |
| No query algorithms, codecs, interaction state or scheduling; every export is synchronous. | [architecture: layers](architecture.md#layers), [architecture: cancellation](architecture.md#concurrency-and-cancellation). | Review |
| `apply` returns the same view when nothing changed and keeps the filtered view when only the sort changed; Swift follows `is_same_view`. | Reuse is decided once, next to the data. | `applying_a_query_reuses_what_did_not_change` (FFI), `applyReusesWhatTheQueryDidNotChange` (GewuKit), `sortChangesKeepTheRowSetAndItsAnalyses` (App) |
| The analysis cache keeps one result per analysis and column, never caches failures, and stays bounded by the schema. | Panels changing parameters must not accumulate results. | `a_slot_keeps_one_result_and_recomputes_for_new_parameters` |
| A lent pointer points into memory its UniFFI object owns and never mutates; each lending function consumes exactly one cloned handle. | Memory safety across the boundary ([architecture: data plane](architecture.md#the-data-plane)). | `source_rows_lend_the_views_own_indices`, `ordered_views_lend_their_source_rows`; review of every `unsafe` block and its `SAFETY` comment |
| New bulk data is either a packed value or a typed pointer through `gewu_data.h`; no pointers as integers through UniFFI. | One data plane, with declared types. | Review |
| `ColumnText` layout matches on both sides. | Swift decodes it in place. | `column_text_packs_offsets_validity_and_text`, `columnTextDecodesCellsInPlace` |
| Errors keep structured kinds; only contract violations become `Internal`; a filter failure keeps its `Filter` kind wherever it arises. | [architecture: errors](architecture.md#errors). | `errors_keep_their_structured_kinds`, `filter_failures_inside_analyses_keep_their_filter_kind` |
| Interface changes are validated on both sides. | Rust tests do not compile the generated Swift. | Rust and App CI |

### GewuKit

| Rule | Why | Guarded by |
| --- | --- | --- |
| Only GewuKit imports `GewuBindings` or `gewuFFI`, with `internal import`; no binding type in its public API. | [architecture: GewuKit](architecture.md#gewukit-the-swift-domain-layer). | CI step "Keep the bindings private to GewuKit"; compiler |
| `TableSource`, `RowSet` and `DisplayRows` are immutable and compare by identity. | Identity is what the App keys work on ([architecture: mental model](architecture.md#mental-model)). | Review |
| Filter literals compare by UTF-8 bytes and filters keep column order, so equal queries select the same rows. | `String` equality merges spellings the engine keeps apart. | `literalsCompareByBytesAsTheEngineDoes`, `canonicallyEquivalentValuesAreDifferentQueries`, `filtersKeepOneOrderWhateverOrderTheyWereSetIn`, `exactValuesCompareBytesAndNullsAreIndependent` |
| Every Rust call runs on a serial worker, off the main actor and the cooperative pool; a call cancelled before it starts never enters Rust. | Long scans must not block the UI or other tasks. | `queuedWorkCancelledBeforeItStartsNeverRuns`; review |
| Cancellation from Swift or Rust surfaces as `CancellationError`, never as a `GewuKitError`. | Callers handle cancellation in one way. | `rustCancellationArrivesAsCancellationError` |
| `GewuKitError` carries a kind and the English diagnostic, never user-facing wording. | Wording belongs to the App. | Compiler (exhaustive switches); review |
| Edit rules live in `TableState` and `FilterDraft`. | App models run rules, they don't define them. | `hidingAColumnClearsItsConditionsAndMovesChoicesToVisibleColumns`, `sortsCycleAndKeepPriorityWhateverTheColumnOrder`, `eachModeKeepsItsDraftAndReopensFromItsFilter` |
| One waiter cancelling does not cancel another's analysis. | Panels share results. | `oneWaiterCancellingDoesNotCancelAnother` |

### App

| Rule | Why | Guarded by |
| --- | --- | --- |
| No imports of the bindings and no FFI type names. | [architecture: GewuKit](architecture.md#gewukit-the-swift-domain-layer). | CI; compiler |
| Swift 6 language mode; no `@unchecked Sendable`. | Data-race safety checked by the compiler. | Compiler (`SWIFT_VERSION: '6.0'`); review |
| Each piece of state has one owner whose lifetime matches it; models are `@Observable` and never refer to views. | No duplicated or orphaned state. | Review |
| `displayed` changes only when a query succeeds; a failed query keeps it and returns the draft to it. | Status, headers, Find and analyses describe one result. | `failedQueriesKeepTheDisplayedRowsAndRestoreTheDraft`, `queryEditsPublishOnlyTheirFinalDraft` |
| A cancelled task changes no state when it ends, whether it succeeded or failed. | Superseded work must be invisible. | `revertingAPendingFilterCancelsItsQuery`, `lateReadsForReplacedRowsAreDiscarded`; review of every task handler |
| The table never shows a blank, partial or mixed frame while rows change. | The table is the workspace. | `newRowsWaitUntilEveryVisibleValueIsReady`, `aSupersededSwitchNeverReplacesTheNewerRows`, `testFilterPublishesWholeTableFrames` (run manually) |
| Column visibility and order are presentation: they apply at once and never run a query. | Hiding a column cannot fail. | `showingAndHidingColumnsKeepsRowsAndLoadedText`, `hidingAColumnAppliesAtOnceAndClearsItsConditions` |
| Panels load in `.task(id:)` keyed by result identity, column and parameters. | SwiftUI cancels obsolete work. | Review |
| `ErrorPresentation` is the only place that words engine failures. | One wording, localizable later. | `engineFailuresAreWordedFromTheirKind`; review |
| Copy is exactly a TSV export of the selected rows and visible columns. | [architecture: copy](architecture.md#copy-is-an-export). | `copyPutsTheLatestSelectionOnThePasteboardAsTSV`, `tabSeparatedTextQuotesFieldsAndRespectsItsLimit` |
