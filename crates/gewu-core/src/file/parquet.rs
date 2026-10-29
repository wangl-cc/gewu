//! Parquet-specific decoding, metadata and encoding.

use std::{collections::BTreeSet, fs::File};

use parquet::{
    arrow::{arrow_reader::ParquetRecordBatchReaderBuilder, ArrowWriter},
    basic::Compression,
    file::properties::WriterProperties,
};

use super::{collect_batches, ExportData, LoadedFile, WriteProgress, CHUNK_ROWS};
use crate::{FileMetadata, Result};

pub(super) fn read(file: File) -> Result<LoadedFile> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let meta = builder.metadata();
    let compression: BTreeSet<String> = meta
        .row_groups()
        .iter()
        .flat_map(|rg| rg.columns())
        .map(|c| {
            let s = c.compression().to_string();
            s.split('(').next().unwrap_or(&s).to_owned()
        })
        .collect();
    let metadata = FileMetadata::Parquet {
        row_groups: meta.num_row_groups() as u32,
        compression: compression.into_iter().collect(),
        created_by: meta.file_metadata().created_by().map(str::to_owned),
        key_values: meta
            .file_metadata()
            .key_value_metadata()
            .into_iter()
            .flatten()
            .filter(|kv| kv.key != "ARROW:schema")
            .map(|kv| (kv.key.clone(), kv.value.clone().unwrap_or_default()))
            .collect(),
    };
    let schema = builder.schema().clone();
    let reader = builder.with_batch_size(64 * 1024).build()?;
    Ok(LoadedFile {
        batch: collect_batches(&schema, reader)?,
        metadata,
    })
}

pub(super) fn write(
    file: &mut File,
    data: ExportData<'_>,
    progress: WriteProgress<'_>,
) -> Result<()> {
    let properties = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .set_max_row_group_row_count(Some(64 * 1024))
        .set_max_row_group_bytes(Some(16 * 1024 * 1024))
        .build();
    let mut writer = ArrowWriter::try_new(file, data.schema.clone(), Some(properties))?;
    for chunk in data.rows.chunks(CHUNK_ROWS) {
        progress.check()?;
        writer.write(&data.batch(chunk)?)?;
        progress.advance(chunk.len());
    }
    progress.check()?;
    writer.close()?;
    Ok(())
}
