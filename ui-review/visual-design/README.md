# Gewu visual review

The sixteen scenes below cover the Default and high-contrast palettes on macOS. The links open PNGs in the local workspace. In a fresh checkout, [run the capture driver](#reproduce) to generate the 128 Default matrix images, 32 high-contrast images and two truncated-match views. The two WindowServer references were captured separately.

| Scene | Default 1× | High contrast 2× |
| --- | --- | --- |
| 01 · Document, selection and inspector tabs | [Light](standard/light-active-1x/01-document.png) · [Dark](standard/dark-active-1x/01-document.png) | [Light](high-contrast/light-active-2x/01-document.png) · [Dark](high-contrast/dark-active-2x/01-document.png) |
| 03 · 104 substring matches across two columns | [Light](standard/light-active-1x/03-find-substring.png) · [Dark](standard/dark-active-1x/03-find-substring.png) | [Light](high-contrast/light-active-2x/03-find-substring.png) · [Dark](high-contrast/dark-active-2x/03-find-substring.png) |
| 31b · All 2,000,000 city values match | [Light](standard/light-active-1x/31b-wide-find.png) · [Dark](standard/dark-active-1x/31b-wide-find.png) | [Light](high-contrast/light-active-2x/31b-wide-find.png) · [Dark](high-contrast/dark-active-2x/31b-wide-find.png) |
| 10 · Apply button and checked values | [Light](standard/light-active-1x/10-value-filter.png) · [Dark](standard/dark-active-1x/10-value-filter.png) | [Light](high-contrast/light-active-2x/10-value-filter.png) · [Dark](high-contrast/dark-active-2x/10-value-filter.png) |
| 20 · Hide a sorted column | [Light](standard/light-active-1x/20-hide-confirmation.png) · [Dark](standard/dark-active-1x/20-hide-confirmation.png) | [Light](high-contrast/light-active-2x/20-hide-confirmation.png) · [Dark](high-contrast/dark-active-2x/20-hide-confirmation.png) |
| 21 · Two sort rules and direction controls | [Light](standard/light-active-1x/21-sort.png) · [Dark](standard/dark-active-1x/21-sort.png) | [Light](high-contrast/light-active-2x/21-sort.png) · [Dark](high-contrast/dark-active-2x/21-sort.png) |
| 29 · Row height and high contrast choices | [Light](standard/light-active-1x/29-settings.png) · [Dark](standard/dark-active-1x/29-settings.png) | [Light](high-contrast/light-active-2x/29-settings.png) · [Dark](high-contrast/dark-active-2x/29-settings.png) |
| Log Y off · Linear Y axis | [Light](standard/light-active-1x/log-y-off.png) · [Dark](standard/dark-active-1x/log-y-off.png) | [Light](high-contrast/light-active-2x/log-y-off.png) · [Dark](high-contrast/dark-active-2x/log-y-off.png) |
| Log Y on · Logarithmic Y axis | [Light](standard/light-active-1x/log-y-on.png) · [Dark](standard/dark-active-1x/log-y-on.png) | [Light](high-contrast/light-active-2x/log-y-on.png) · [Dark](high-contrast/dark-active-2x/log-y-on.png) |
| 18 · Column header context menu | [Light](standard/light-active-1x/18-header-menu.png) · [Dark](standard/dark-active-1x/18-header-menu.png) | [Light](high-contrast/light-active-2x/18-header-menu.png) · [Dark](high-contrast/dark-active-2x/18-header-menu.png) |
| 23 · Cell context menu | [Light](standard/light-active-1x/23-cell-menu.png) · [Dark](standard/dark-active-1x/23-cell-menu.png) | [Light](high-contrast/light-active-2x/23-cell-menu.png) · [Dark](high-contrast/dark-active-2x/23-cell-menu.png) |
| 19b · Secondary buttons with a column-name query | [Light](standard/light-active-1x/19b-visible-columns.png) · [Dark](standard/dark-active-1x/19b-visible-columns.png) | [Light](high-contrast/light-active-2x/19b-visible-columns.png) · [Dark](high-contrast/dark-active-2x/19b-visible-columns.png) |
| 16b · Active Station filter at 680 pt | [Light](standard/light-active-1x/16b-filter-summary.png) · [Dark](standard/dark-active-1x/16b-filter-summary.png) | [Light](high-contrast/light-active-2x/16b-filter-summary.png) · [Dark](high-contrast/dark-active-2x/16b-filter-summary.png) |
| 24 · Full cell value and Copy | [Light](standard/light-active-1x/24-cell-preview.png) · [Dark](standard/dark-active-1x/24-cell-preview.png) | [Light](high-contrast/light-active-2x/24-cell-preview.png) · [Dark](high-contrast/dark-active-2x/24-cell-preview.png) |
| 30b · Expanded Parquet diagnostic | [Light](standard/light-active-1x/30b-open-error.png) · [Dark](standard/dark-active-1x/30b-open-error.png) | [Light](high-contrast/light-active-2x/30b-open-error.png) · [Dark](high-contrast/dark-active-2x/30b-open-error.png) |
| 14 · Brushed histogram and range inputs at 680 pt | [Light](standard/light-active-1x/14-range-filter.png) · [Dark](standard/dark-active-1x/14-range-filter.png) | [Light](high-contrast/light-active-2x/14-range-filter.png) · [Dark](high-contrast/dark-active-2x/14-range-filter.png) |

The truncated-match pair narrows Remarks to 76 pt and searches for `transect`. Rows 1, 7 and 13 display “Mornin…” with the amber span confined to the ellipsis. Row 1 also has the current-match outline. [Light](standard/light-active-1x/03b-truncated-ellipsis.png) · [Dark](standard/dark-active-1x/03b-truncated-ellipsis.png)

The Log Y pair selects the point for row 81 (water temperature 16.4) with each Y-axis scale. The plot and the table show the same selected record.

## Window states and scales

| Default | Active 1× | Inactive 1× | Active 2× | Inactive 2× |
| --- | --- | --- | --- | --- |
| Light | [Open](standard/light-active-1x/) | [Open](standard/light-inactive-1x/) | [Open](standard/light-active-2x/) | [Open](standard/light-inactive-2x/) |
| Dark | [Open](standard/dark-active-1x/) | [Open](standard/dark-inactive-1x/) | [Open](standard/dark-active-2x/) | [Open](standard/dark-inactive-2x/) |

Scenes 18 and 23 show the menu in the active window. When the window loses activation, Gewu dismisses the menu. The corresponding inactive screenshots capture that transition. For scene 31b, the counter has settled at 1/2,000,000. The result highlights remain visible across the current viewport.

Pixel sampling of the Apply button in scene 10 confirms the Default fills are exactly `#007AFF` (light) and `#0A84FF` (dark), with pure white inside the text. The corresponding high-contrast fills are `#005BBC` and `#0070D6`.

A held click on the text or 2 pt from either edge of Sort Ascending filled the same full menu row. A scan through its interior returned 182 consecutive pixels of `#DBDBDB` at light 1× and 364 of `#565658` at dark 2×. Each of the six releases executed the menu command once.

## Capture method

The matrix uses the production document views, Apple text system, bundled icons and GPUI Metal renderer. The capture host opens the CSV or Parquet through Gewu's document loader, sends input to the rendered controls, and supplies window activation, appearance, logical size and backing scale through GPUI's headless test platform. Each capture checks the palette, window state, accessible control and PNG dimensions. At 2× the application draws directly into a texture with twice the width and height. The host captures the content area, which includes Gewu's title and toolbar.

Two additional 1× images include the title-bar controls composited by the macOS WindowServer. ScreenCaptureKit selected the review process's own document window. The sole physical display was 1920×1080 at 1× and the macOS session was locked. Both native captures reported an inactive application and a non-key window. The active-window and 2× coverage in the matrix comes from GPUI's controlled rendering environment. [Light native window](native-reference/light-inactive-1x.png) · [Dark native window](native-reference/dark-inactive-1x.png)

## Reproduce

On macOS with Metal and the repository toolchain available, run the following from the checkout.

```sh
ruby ui-review/visual-design/driver/capture.rb
```

The driver builds its own entry point below Cargo's target directory using the checked-out sources and locked dependencies. Its adapter for the pinned GPUI version delivers output-device changes through the test-platform callbacks and exposes the completed accessibility tree. AccessKit supplies each control’s computed name and bounds. The review process uses session-local document history and preferences. For high contrast, it opens Settings and selects On before capturing the document. If system-wide Increase Contrast is already on, the standard run selects Off in this isolated session.

The small fixture contains 96 estuary observations. The driver writes a deliberately incomplete Parquet file for the open-error scene, and runs the repository's seeded generator to create the two-million-row Parquet when needed. An existing file from `gewu-data-gen` can be supplied with `--sample`. Use `--output` to save a comparison run elsewhere, and `--group` to repeat specific scenes.

```sh
ruby ui-review/visual-design/driver/capture.rb --mode standard --group truncated --output /tmp/gewu-ellipsis
ruby ui-review/visual-design/driver/capture.rb --mode high-contrast --group range --output /tmp/gewu-range
```
