# Gewu visual review

The sixteen scenes below cover the Default and high-contrast palettes on macOS at 2× Retina resolution. By default, captures are written under `target/ui-capture/`. In a fresh checkout, [run the capture driver](#reproduce) to generate the 64 Default matrix images, 32 high-contrast images and two truncated-match views.

| Scene | Default 2× | High contrast 2× |
| --- | --- | --- |
| 01 · Document, selection and inspector tabs | [Light](../../target/ui-capture/standard/light-active-2x/01-document.png) · [Dark](../../target/ui-capture/standard/dark-active-2x/01-document.png) | [Light](../../target/ui-capture/high-contrast/light-active-2x/01-document.png) · [Dark](../../target/ui-capture/high-contrast/dark-active-2x/01-document.png) |
| 03 · 104 substring matches across two columns | [Light](../../target/ui-capture/standard/light-active-2x/03-find-substring.png) · [Dark](../../target/ui-capture/standard/dark-active-2x/03-find-substring.png) | [Light](../../target/ui-capture/high-contrast/light-active-2x/03-find-substring.png) · [Dark](../../target/ui-capture/high-contrast/dark-active-2x/03-find-substring.png) |
| 31b · All 2,000,000 city values match | [Light](../../target/ui-capture/standard/light-active-2x/31b-wide-find.png) · [Dark](../../target/ui-capture/standard/dark-active-2x/31b-wide-find.png) | [Light](../../target/ui-capture/high-contrast/light-active-2x/31b-wide-find.png) · [Dark](../../target/ui-capture/high-contrast/dark-active-2x/31b-wide-find.png) |
| 10 · Apply button and checked values | [Light](../../target/ui-capture/standard/light-active-2x/10-value-filter.png) · [Dark](../../target/ui-capture/standard/dark-active-2x/10-value-filter.png) | [Light](../../target/ui-capture/high-contrast/light-active-2x/10-value-filter.png) · [Dark](../../target/ui-capture/high-contrast/dark-active-2x/10-value-filter.png) |
| 20 · Hide a sorted column | [Light](../../target/ui-capture/standard/light-active-2x/20-hide-confirmation.png) · [Dark](../../target/ui-capture/standard/dark-active-2x/20-hide-confirmation.png) | [Light](../../target/ui-capture/high-contrast/light-active-2x/20-hide-confirmation.png) · [Dark](../../target/ui-capture/high-contrast/dark-active-2x/20-hide-confirmation.png) |
| 21 · Sort first by Source, then by Water temperature | [Light](../../target/ui-capture/standard/light-active-2x/21-sort.png) · [Dark](../../target/ui-capture/standard/dark-active-2x/21-sort.png) | [Light](../../target/ui-capture/high-contrast/light-active-2x/21-sort.png) · [Dark](../../target/ui-capture/high-contrast/dark-active-2x/21-sort.png) |
| 29 · Row height and high contrast choices | [Light](../../target/ui-capture/standard/light-active-2x/29-settings.png) · [Dark](../../target/ui-capture/standard/dark-active-2x/29-settings.png) | [Light](../../target/ui-capture/high-contrast/light-active-2x/29-settings.png) · [Dark](../../target/ui-capture/high-contrast/dark-active-2x/29-settings.png) |
| Log Y off · Linear Y axis | [Light](../../target/ui-capture/standard/light-active-2x/log-y-off.png) · [Dark](../../target/ui-capture/standard/dark-active-2x/log-y-off.png) | [Light](../../target/ui-capture/high-contrast/light-active-2x/log-y-off.png) · [Dark](../../target/ui-capture/high-contrast/dark-active-2x/log-y-off.png) |
| Log Y on · Logarithmic Y axis | [Light](../../target/ui-capture/standard/light-active-2x/log-y-on.png) · [Dark](../../target/ui-capture/standard/dark-active-2x/log-y-on.png) | [Light](../../target/ui-capture/high-contrast/light-active-2x/log-y-on.png) · [Dark](../../target/ui-capture/high-contrast/dark-active-2x/log-y-on.png) |
| 18 · Column header context menu | [Light](../../target/ui-capture/standard/light-active-2x/18-header-menu.png) · [Dark](../../target/ui-capture/standard/dark-active-2x/18-header-menu.png) | [Light](../../target/ui-capture/high-contrast/light-active-2x/18-header-menu.png) · [Dark](../../target/ui-capture/high-contrast/dark-active-2x/18-header-menu.png) |
| 23 · Cell context menu | [Light](../../target/ui-capture/standard/light-active-2x/23-cell-menu.png) · [Dark](../../target/ui-capture/standard/dark-active-2x/23-cell-menu.png) | [Light](../../target/ui-capture/high-contrast/light-active-2x/23-cell-menu.png) · [Dark](../../target/ui-capture/high-contrast/dark-active-2x/23-cell-menu.png) |
| 19b · Secondary buttons with a column-name query | [Light](../../target/ui-capture/standard/light-active-2x/19b-visible-columns.png) · [Dark](../../target/ui-capture/standard/dark-active-2x/19b-visible-columns.png) | [Light](../../target/ui-capture/high-contrast/light-active-2x/19b-visible-columns.png) · [Dark](../../target/ui-capture/high-contrast/dark-active-2x/19b-visible-columns.png) |
| 16b · Active Station filter at 680 pt | [Light](../../target/ui-capture/standard/light-active-2x/16b-filter-summary.png) · [Dark](../../target/ui-capture/standard/dark-active-2x/16b-filter-summary.png) | [Light](../../target/ui-capture/high-contrast/light-active-2x/16b-filter-summary.png) · [Dark](../../target/ui-capture/high-contrast/dark-active-2x/16b-filter-summary.png) |
| 24 · Full cell value and Copy | [Light](../../target/ui-capture/standard/light-active-2x/24-cell-preview.png) · [Dark](../../target/ui-capture/standard/dark-active-2x/24-cell-preview.png) | [Light](../../target/ui-capture/high-contrast/light-active-2x/24-cell-preview.png) · [Dark](../../target/ui-capture/high-contrast/dark-active-2x/24-cell-preview.png) |
| 30b · Expanded Parquet diagnostic | [Light](../../target/ui-capture/standard/light-active-2x/30b-open-error.png) · [Dark](../../target/ui-capture/standard/dark-active-2x/30b-open-error.png) | [Light](../../target/ui-capture/high-contrast/light-active-2x/30b-open-error.png) · [Dark](../../target/ui-capture/high-contrast/dark-active-2x/30b-open-error.png) |
| 14 · Brushed histogram and range inputs at 680 pt | [Light](../../target/ui-capture/standard/light-active-2x/14-range-filter.png) · [Dark](../../target/ui-capture/standard/dark-active-2x/14-range-filter.png) | [Light](../../target/ui-capture/high-contrast/light-active-2x/14-range-filter.png) · [Dark](../../target/ui-capture/high-contrast/dark-active-2x/14-range-filter.png) |

The truncated-match pair narrows Remarks to 76 pt and searches for `transect`. Rows 1, 7 and 13 display “Mornin…” with the amber span confined to the ellipsis. Row 1 also has the current-match outline. [Light](../../target/ui-capture/standard/light-active-2x/03b-truncated-ellipsis.png) · [Dark](../../target/ui-capture/standard/dark-active-2x/03b-truncated-ellipsis.png)

## Window states

| Default 2× | Active | Inactive |
| --- | --- | --- |
| Light | [Open](../../target/ui-capture/standard/light-active-2x/) | [Open](../../target/ui-capture/standard/light-inactive-2x/) |
| Dark | [Open](../../target/ui-capture/standard/dark-active-2x/) | [Open](../../target/ui-capture/standard/dark-inactive-2x/) |

Scenes 18 and 23 show the menu in the active window. When focus moves to an auxiliary inactive window, Gewu dismisses the menu, and the corresponding inactive screenshots verify that transition.

## Reproduce

On macOS with Metal and the repository toolchain available, run the following from the checkout.

```sh
cargo run --locked -p gewu-ui-capture
```

The tool uses GPUI's public `HeadlessAppContext`, Apple text system, bundled icons and Metal renderer with no GPUI patch or custom `Platform` wrapper, and writes 2× Retina screenshots to `target/ui-capture/`. Controls are driven via `gpui-kit`'s `TestWindowExt` and `TestQueryExt` (`find`, `elements`) and light/dark palettes are installed via `capture::set_appearance`. The review process uses session-local document history and preferences. For high contrast, it opens Settings and selects On before capturing the document; if system-wide Increase Contrast is already on, the standard run selects Off in this isolated session.

The small fixture contains 96 estuary observations. The runner writes a deliberately incomplete Parquet file for the open-error scene, and runs the repository's seeded generator (`gewu-data-gen`) to create the two-million-row Parquet when needed. An existing file from `gewu-data-gen` can be supplied with `--sample`. Use `--output` to save a comparison run elsewhere, and `--group` to repeat specific scenes.

```sh
cargo run --locked -p gewu-ui-capture -- --mode standard --group truncated --output /tmp/gewu-ellipsis
cargo run --locked -p gewu-ui-capture -- --mode high-contrast --group range --output /tmp/gewu-range
```
