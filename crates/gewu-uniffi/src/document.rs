//! App-facing document operations; data and execution remain in the core.

use std::sync::Arc;

use gewu_core as core;

use crate::{
    AnalysisSection, AxisScale, Bound, ColumnAnalysis, ColumnSummary, FileInfo, FileType, Filter,
    FilterSpec, GewuError, GewuExport, Histogram, RowBlock, TextChoices, ViewInfo, ViewState,
};

/// One open file.
#[derive(uniffi::Object)]
pub struct GewuDocument {
    inner: core::Document,
}

#[uniffi::export]
impl GewuDocument {
    /// Capture a bounded scatter job over the current view; stale versions fail.
    /// A nil X column means source row IDs. Limit must be in 1...4096.
    pub fn prepare_scatter(
        &self,
        version: u64,
        x_column: Option<u32>,
        y_column: u32,
        limit: u32,
        x_scale: AxisScale,
        y_scale: AxisScale,
    ) -> Result<Arc<crate::ScatterAnalysis>, GewuError> {
        Ok(Arc::new(crate::ScatterAnalysis {
            inner: self
                .inner
                .prepare_scatter(version, x_column, y_column, limit)?
                .with_scales(x_scale.into(), y_scale.into())?,
        }))
    }

    /// Open a supported table file; XLSX defaults to its first worksheet.
    /// Slow: call off the main thread.
    #[uniffi::constructor]
    pub fn open(path: String) -> Result<Arc<Self>, GewuError> {
        Ok(Arc::new(Self {
            inner: core::Document::open(path)?,
        }))
    }

    /// Open CSV, TSV or XLSX with an explicit header interpretation.
    /// Parquet ignores has_header because its schema already contains names.
    #[uniffi::constructor]
    pub fn open_with_options(
        path: String,
        sheet: Option<String>,
        has_header: bool,
    ) -> Result<Arc<Self>, GewuError> {
        let options = core::OpenOptions {
            sheet,
            has_header,
            ..core::OpenOptions::default()
        };
        Ok(Arc::new(Self {
            inner: core::Document::open_with_options(path, &options)?,
        }))
    }

    /// Open a named XLSX worksheet as an independent document with a fresh view.
    /// Uses the first occupied row as headers. Sheet names match exactly;
    /// a missing sheet or a non-XLSX file is rejected. Slow, non-cancellable I/O:
    /// call off the main thread and discard obsolete results in the caller.
    #[uniffi::constructor]
    pub fn open_sheet(path: String, sheet: String) -> Result<Arc<Self>, GewuError> {
        let options = core::OpenOptions {
            sheet: Some(sheet),
            ..core::OpenOptions::default()
        };
        Ok(Arc::new(Self {
            inner: core::Document::open_with_options(path, &options)?,
        }))
    }

    pub fn info(&self) -> FileInfo {
        (&self.inner).into()
    }

    pub fn current_view(&self) -> ViewInfo {
        self.inner.current_view().into()
    }

    /// Capture the current result and displayed column order without copying
    /// cells. Later queries do not change this export's contents.
    pub fn prepare_export(
        &self,
        version: u64,
        columns: Vec<u32>,
        format: FileType,
    ) -> Result<Arc<GewuExport>, GewuError> {
        Ok(Arc::new(
            self.inner
                .prepare_export(version, &columns, format.into())?
                .into(),
        ))
    }

    pub fn nested_export_columns(&self, columns: Vec<u32>) -> Result<Vec<String>, GewuError> {
        Ok(self.inner.nested_export_columns(&columns)?)
    }

    /// Sort and filter. Slow: call off the main thread. A newer call makes
    /// this one return `Cancelled`.
    pub fn apply_view(&self, state: ViewState) -> Result<ViewInfo, GewuError> {
        Ok(self.inner.apply_view(&state.into())?.into())
    }

    pub fn cancel_pending(&self) {
        self.inner.cancel_pending();
    }

    /// Formatted cells for display rows `start..start+count`. Fast.
    pub fn rows(
        &self,
        version: u64,
        start: u64,
        count: u32,
        columns: Vec<u32>,
    ) -> Result<RowBlock, GewuError> {
        Ok(self.inner.rows(version, start, count, &columns)?.into())
    }

    /// Cheap type metadata: no column scan or cardinality calculation.
    pub fn analysis_capabilities(&self, column: u32) -> Result<Vec<AnalysisSection>, GewuError> {
        Ok(self
            .inner
            .analysis_capabilities(column)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Capture the current view without computing the section. Safe on the UI thread.
    pub fn prepare_analysis(
        &self,
        version: u64,
        column: u32,
        section: AnalysisSection,
    ) -> Result<Arc<ColumnAnalysis>, GewuError> {
        Ok(Arc::new(ColumnAnalysis {
            inner: self
                .inner
                .prepare_analysis(version, column, section.into())?,
        }))
    }

    /// Capture a section with a caller-selected histogram resolution.
    pub fn prepare_analysis_with_bins(
        &self,
        version: u64,
        column: u32,
        section: AnalysisSection,
        bins: u32,
        scale: AxisScale,
    ) -> Result<Arc<ColumnAnalysis>, GewuError> {
        Ok(Arc::new(ColumnAnalysis {
            inner: self
                .inner
                .prepare_analysis(version, column, section.into())?
                .with_histogram_bins(bins)?
                .with_histogram_scale(scale.into())?,
        }))
    }

    /// Distribution over the current displayed rows, with all filters applied.
    /// Call off the main thread.
    pub fn view_histogram(
        &self,
        version: u64,
        column: u32,
        bins: u32,
    ) -> Result<Histogram, GewuError> {
        Ok(self.inner.view_histogram(version, column, bins)?.into())
    }

    /// Display text for a raw value of a date, time or timestamp column
    /// (as in histogram axes and range bounds), formatted like its cells.
    pub fn format_value(&self, column: u32, value: i64) -> Result<String, GewuError> {
        Ok(self.inner.format_value(column, value)?)
    }

    /// The core selects the editor and supplies its values or typed histogram bins.
    /// Slow: call off the main thread.
    pub fn filter_spec(
        &self,
        column: u32,
        state: Option<ViewState>,
    ) -> Result<FilterSpec, GewuError> {
        let state = state.map(Into::into);
        Ok(self.inner.filter_spec(column, state.as_ref())?.into())
    }

    /// Filter editor with a caller-selected histogram resolution (1 through 60).
    pub fn filter_spec_with_bins(
        &self,
        column: u32,
        state: Option<ViewState>,
        bins: u32,
        scale: AxisScale,
    ) -> Result<FilterSpec, GewuError> {
        let state = state.map(Into::into);
        Ok(self
            .inner
            .filter_spec_with_scale(column, state.as_ref(), bins, scale.into())?
            .into())
    }

    /// Compute the facet off the UI thread. The resulting paging session has
    /// independent cancellation; preparation itself completes before it can be cancelled.
    pub fn prepare_text_choices(
        &self,
        column: u32,
        state: Option<ViewState>,
    ) -> Result<Arc<TextChoices>, GewuError> {
        let state = state.map(Into::into);
        Ok(Arc::new(TextChoices {
            inner: self.inner.prepare_text_choices(column, state.as_ref())?,
        }))
    }

    pub fn parse_bound(&self, column: u32, text: String) -> Result<Arc<Bound>, GewuError> {
        Ok(Arc::new(self.inner.parse_bound(column, &text)?.into()))
    }

    pub fn make_bound_int(&self, column: u32, value: i64) -> Result<Arc<Bound>, GewuError> {
        Ok(Arc::new(self.inner.make_bound_i64(column, value)?.into()))
    }

    pub fn make_bound_uint(&self, column: u32, value: u64) -> Result<Arc<Bound>, GewuError> {
        Ok(Arc::new(self.inner.make_bound_u64(column, value)?.into()))
    }

    pub fn make_bound_float(&self, column: u32, value: f64) -> Result<Arc<Bound>, GewuError> {
        Ok(Arc::new(self.inner.make_bound_f64(column, value)?.into()))
    }

    /// Decimal coefficient transported as an integer string to preserve all digits.
    pub fn make_bound_decimal(
        &self,
        column: u32,
        coefficient: String,
        scale: i8,
    ) -> Result<Arc<Bound>, GewuError> {
        Ok(Arc::new(
            self.inner
                .make_bound_decimal(column, &coefficient, scale)?
                .into(),
        ))
    }

    /// Validate a filter without changing the current view.
    pub fn validate_filter(&self, filter: Filter) -> Result<(), GewuError> {
        Ok(self.inner.validate_filter(&filter.into())?)
    }

    /// Summary of a column over the current view (filters and search apply).
    /// Call off the main thread.
    pub fn column_summary(&self, column: u32, top: u32) -> Result<ColumnSummary, GewuError> {
        Ok(self.inner.column_summary(column, top)?.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FilterEditor, TextFilterMode};

    #[test]
    fn typed_filters_validate_and_execute_across_ffi() {
        let source =
            std::env::temp_dir().join(format!("gewu-uniffi-typed-{}.csv", std::process::id()));
        std::fs::write(&source, "id\n9007199254740992\n9007199254740993\n").unwrap();
        let doc = GewuDocument::open(source.to_str().unwrap().to_owned()).unwrap();
        assert!(doc.info().columns[0].capabilities.histogram);
        assert!(doc.info().columns[0].capabilities.scatter);
        let scatter = doc
            .prepare_scatter(0, None, 0, 4, AxisScale::Linear, AxisScale::Linear)
            .unwrap();
        let points = scatter.run().unwrap();
        assert_eq!(points.eligible_rows, 2);
        assert!(!points.sampled);
        assert_eq!(points.points[1].source_row, 1);
        assert_eq!(points.points[1].y_label, "9007199254740993");
        scatter.cancel();
        assert!(matches!(scatter.run(), Err(GewuError::Cancelled)));
        let stats = doc
            .prepare_analysis(0, 0, AnalysisSection::Statistics)
            .unwrap()
            .run()
            .unwrap();
        let crate::AnalysisData::Statistics { statistics } = stats.data else {
            panic!("statistics");
        };
        assert_eq!(
            statistics.median_label.as_deref(),
            Some("9007199254740992.5")
        );
        assert!(!doc.info().columns[0].capabilities.contains);
        // Chart coordinates may round a large ID; typed filters below must not.
        let large = doc.parse_bound(0, "9007199254740993".into()).unwrap();
        assert_eq!(large.display(), "9007199254740993");
        assert_eq!(large.axis_value(), 9_007_199_254_740_992.0);
        assert!(large.same_value(doc.make_bound_int(0, 9_007_199_254_740_993).unwrap()));
        assert!(!large.same_value(doc.make_bound_int(0, 9_007_199_254_740_992).unwrap()));
        assert!(matches!(
            doc.parse_bound(0, "NaN".into()),
            Err(GewuError::InvalidArgument { .. })
        ));
        let before = doc.current_view();
        assert!(matches!(
            doc.parse_bound(0, "not a number".into()),
            Err(GewuError::InvalidArgument { .. })
        ));
        assert!(matches!(
            doc.filter_spec(0, None).unwrap().editor,
            FilterEditor::Choices { .. }
        ));
        assert_eq!(doc.current_view().version, before.version);

        for filter in [
            Filter::Equals {
                column: 0,
                value: "9007199254740993".into(),
            },
            Filter::Range {
                include_nulls: false,
                column: 0,
                min: Some(large),
                max: None,
            },
            Filter::Range {
                include_nulls: false,
                column: 0,
                min: Some(doc.make_bound_int(0, 9_007_199_254_740_993).unwrap()),
                max: None,
            },
        ] {
            let view = doc
                .apply_view(ViewState {
                    sort: Vec::new(),
                    filters: vec![filter],
                    search: None,
                })
                .unwrap();
            let rows = doc.rows(view.version, 0, 10, vec![0]).unwrap();
            assert_eq!(rows.source_rows, [1]);
            assert_eq!(rows.cells, [Some("9007199254740993".into())]);
        }
        std::fs::remove_file(source).unwrap();
    }

    #[test]
    fn text_filter_modes_and_paged_choices_cross_ffi() {
        let source = std::env::temp_dir().join(format!(
            "gewu-uniffi-filter-spec-{}.csv",
            std::process::id()
        ));
        std::fs::write(&source, "label,id\ncat,10\ndog,20\n").unwrap();
        let doc = GewuDocument::open(source.to_str().unwrap().to_owned()).unwrap();
        let FilterEditor::Text { modes, recommended } = doc.filter_spec(0, None).unwrap().editor
        else {
            panic!("strings should offer both filter modes");
        };
        assert!(matches!(modes.as_slice(), [
            TextFilterMode::Choices,
            TextFilterMode::Contains
        ]));
        assert!(matches!(recommended, TextFilterMode::Choices));
        let state = ViewState {
            sort: vec![],
            filters: vec![Filter::Equals {
                column: 1,
                value: "10".into(),
            }],
            search: None,
        };
        let choices = doc.prepare_text_choices(0, Some(state)).unwrap();
        let first = choices.page(String::new(), None, 1).unwrap();
        assert_eq!(first.values.len(), 1);
        assert_eq!(first.values[0].value.as_deref(), Some("cat"));
        assert_eq!(first.values[0].count, 1);
        let second = choices.page(String::new(), first.next, 1).unwrap();
        assert_eq!(second.values.len(), 1);
        assert_eq!(second.values[0].value.as_deref(), Some("dog"));
        assert_eq!(second.values[0].count, 0);
        assert!(second.next.is_none());
        let view = doc
            .apply_view(ViewState {
                sort: vec![],
                filters: vec![Filter::TextValues {
                    column: 0,
                    values: vec![Some("dog".into())],
                    exclude: false,
                }],
                search: None,
            })
            .unwrap();
        assert_eq!(doc.rows(view.version, 0, 10, vec![0]).unwrap().cells, [
            Some("dog".into())
        ]);
        choices.cancel();
        assert!(matches!(
            choices.page(String::new(), None, 1),
            Err(GewuError::Cancelled)
        ));
        std::fs::remove_file(source).unwrap();
    }

    #[test]
    fn histogram_bounds_cross_ffi_without_losing_large_ids() {
        use std::fmt::Write;

        let source =
            std::env::temp_dir().join(format!("gewu-uniffi-range-spec-{}.csv", std::process::id()));
        let base = 9_007_199_254_740_993_i64;
        let mut csv = String::from("id\n");
        for offset in 0..1_000 {
            writeln!(csv, "{}", base + offset).unwrap();
        }
        std::fs::write(&source, csv).unwrap();
        let doc = GewuDocument::open(source.to_str().unwrap().to_owned()).unwrap();
        let FilterEditor::Range {
            distribution: Some(custom),
            ..
        } = doc
            .filter_spec_with_bins(0, None, 10, AxisScale::Linear)
            .unwrap()
            .editor
        else {
            panic!("custom histogram");
        };
        assert_eq!(custom.histogram.counts.len(), 10);
        assert_eq!(custom.histogram.counts.iter().sum::<u64>(), 1_000);
        assert!(matches!(
            doc.filter_spec_with_bins(0, None, 0, AxisScale::Linear),
            Err(GewuError::InvalidArgument { .. })
        ));
        let FilterEditor::Range {
            distribution: Some(distribution),
            ..
        } = doc.filter_spec(0, None).unwrap().editor
        else {
            panic!("many distinct integers should offer a typed range histogram");
        };
        assert_eq!(distribution.histogram.min_label, Some(base.to_string()));
        assert_eq!(
            distribution.histogram.max_label,
            Some((base + 999).to_string())
        );
        assert_eq!(
            distribution.ranges.len(),
            distribution.histogram.counts.len()
        );
        assert!(distribution.ranges.first().unwrap().min.is_none());
        assert!(distribution.ranges.last().unwrap().max.is_none());
        for bin in [
            0,
            distribution.ranges.len() / 2,
            distribution.ranges.len() - 1,
        ] {
            let bounds = &distribution.ranges[bin];
            let view = doc
                .apply_view(ViewState {
                    sort: vec![],
                    filters: vec![Filter::Range {
                        include_nulls: false,
                        column: 0,
                        min: bounds.min.clone(),
                        max: bounds.max.clone(),
                    }],
                    search: None,
                })
                .unwrap();
            assert_eq!(view.row_count, distribution.histogram.counts[bin]);
        }
        std::fs::remove_file(source).unwrap();
    }

    #[test]
    fn file_metadata_and_xlsx_export_cross_the_ffi_boundary() {
        let source =
            std::env::temp_dir().join(format!("gewu-uniffi-formats-{}.csv", std::process::id()));
        std::fs::write(&source, "label,id\ncat,10\ndog,20\n").unwrap();
        let doc = GewuDocument::open(source.to_str().unwrap().to_owned()).unwrap();
        let info = doc.info();
        assert_eq!(info.format, "CSV");
        assert_eq!(info.row_count, 2);
        assert_eq!(info.row_groups, None);
        assert!(info.compression.is_empty());
        assert!(info.sheet.is_none());
        assert!(matches!(
            crate::xlsx_sheet_names(source.to_str().unwrap().to_owned()),
            Err(GewuError::InvalidArgument { .. })
        ));
        assert!(matches!(
            GewuDocument::open_sheet(source.to_str().unwrap().to_owned(), "Sheet1".into()),
            Err(GewuError::InvalidArgument { .. })
        ));

        for (format, extension, name) in [
            (FileType::Xlsx, "xlsx", "XLSX"),
            (FileType::Parquet, "parquet", "Parquet"),
        ] {
            let destination = source.with_extension(extension);
            doc.prepare_export(doc.current_view().version, vec![0, 1], format)
                .unwrap()
                .write(destination.to_str().unwrap().to_owned())
                .unwrap();
            let reopened = GewuDocument::open(destination.to_str().unwrap().to_owned()).unwrap();
            let info = reopened.info();
            assert_eq!(info.format, name);
            assert_eq!(info.row_count, 2);
            if extension == "xlsx" {
                assert_eq!(info.row_groups, None);
                let sheet = info.sheet.unwrap();
                let path = destination.to_str().unwrap().to_owned();
                assert_eq!(crate::xlsx_sheet_names(path.clone()).unwrap(), info.sheets);
                assert_eq!(info.sheets, vec![sheet.clone()]);
                let selected = GewuDocument::open_sheet(path.clone(), sheet.clone()).unwrap();
                assert_eq!(selected.info().sheet, Some(sheet));
                assert_eq!(selected.rows(0, 0, 1, vec![0]).unwrap().cells, [Some(
                    "cat".into()
                )]);
                selected
                    .apply_view(ViewState {
                        sort: vec![],
                        filters: vec![Filter::Equals {
                            column: 0,
                            value: "cat".into(),
                        }],
                        search: None,
                    })
                    .unwrap();
                assert_eq!(selected.current_view().row_count, 1);
                assert_eq!(reopened.current_view().row_count, 2);
                assert!(matches!(
                    GewuDocument::open_sheet(path, "absent".into()),
                    Err(GewuError::InvalidArgument { .. })
                ));
            } else {
                assert_eq!(info.row_groups, Some(1));
                assert!(!info.compression.is_empty());
                assert!(info.sheet.is_none());
            }
            std::fs::remove_file(destination).unwrap();
        }
        std::fs::remove_file(source).unwrap();
    }
}
