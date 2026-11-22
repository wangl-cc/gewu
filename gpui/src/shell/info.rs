//! An information window owns only a description of the file as it was opened.
use std::path::PathBuf;

use gewu_core::{DatasetInfo, FileMetadata, FileType};
use gpui_kit::{base::Theme, *};

use crate::{panels::button, platform};

struct FileDetails {
    path: PathBuf,
    rows: Vec<(String, String)>,
}

impl From<DatasetInfo> for FileDetails {
    fn from(info: DatasetInfo) -> Self {
        let path = PathBuf::from(info.path);
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned();
        let kind = match info.format {
            FileType::Csv => "CSV",
            FileType::Tsv => "TSV",
            FileType::Xlsx => "Excel",
            FileType::Parquet => "Parquet",
        };
        let mut rows = vec![
            ("Name".into(), name),
            ("Kind".into(), format!("{kind} table")),
            ("Size".into(), format!("{} bytes", info.file_size)),
            ("Rows".into(), info.row_count.to_string()),
            ("Columns".into(), info.columns.len().to_string()),
            ("Location".into(), path.display().to_string()),
        ];
        match info.metadata {
            FileMetadata::Parquet {
                row_groups,
                compression,
                created_by,
                key_values,
            } => {
                rows.push(("Row groups".into(), row_groups.to_string()));
                rows.push((
                    "Compression".into(),
                    if compression.is_empty() {
                        "None".into()
                    } else {
                        compression.join(", ")
                    },
                ));
                if let Some(writer) = created_by {
                    rows.push(("Created by".into(), writer));
                }
                rows.extend(key_values);
            }
            FileMetadata::Xlsx { sheet, sheets } => {
                rows.push(("Worksheet".into(), sheet));
                rows.push(("Workbook sheets".into(), sheets.join(", ")));
            }
            FileMetadata::Delimited => {}
        }
        Self { path, rows }
    }
}

pub(super) fn open(info: DatasetInfo, cx: &mut App) {
    let details = FileDetails::from(info);
    let options = platform::sized_window(470., 430., cx);
    if let Err(error) = gpui_kit::open_window(options, cx, |window, cx| {
        platform::follow_appearance(window, cx);
        window.set_window_title(&format!(
            "{} Info",
            details
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        ));
        cx.new(|_| details)
    }) {
        super::report_window_error(error.to_string(), cx);
    }
}

impl Render for FileDetails {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let mut view = div()
            .id("file-info")
            .size_full()
            .overflow_y_scroll()
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .bg(colors.background)
            .text_color(colors.foreground);
        for (key, value) in &self.rows {
            view = view.child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        div()
                            .w(px(112.))
                            .flex_none()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(key.clone()),
                    )
                    .child(div().min_w_0().flex_1().child(value.clone())),
            );
        }
        view.child(
            button("reveal-file", "Show in Finder", false, cx)
                .on_click(cx.listener(|this, _, _, cx| cx.reveal_path(&this.path))),
        )
    }
}
