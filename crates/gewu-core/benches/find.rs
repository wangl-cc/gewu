//! Bounded Find scans over representative dense, sparse and fallback inputs.

use std::sync::Arc;

use arrow::{
    array::{
        ArrayRef, AsArray, BooleanArray, DictionaryArray, Int32Array, IntervalYearMonthArray,
        ListArray, RecordBatch, StringArray,
    },
    buffer::OffsetBuffer,
    datatypes::{DataType, Field, Int32Type},
};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use gewu_core::{Document, Filter, SortKey, ViewState};
use parquet::arrow::{arrow_reader::ParquetRecordBatchReaderBuilder, ArrowWriter};

fn document(columns: Vec<(&str, ArrayRef)>) -> (tempfile::NamedTempFile, Document) {
    let batch = RecordBatch::try_from_iter(columns).unwrap();
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut writer = ArrowWriter::try_new(file.reopen().unwrap(), batch.schema(), None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
    let document = Document::open(file.path()).unwrap();
    (file, document)
}

fn find_scans(c: &mut Criterion) {
    let mut group = c.benchmark_group("find_scan");
    group.sample_size(10);

    let text: ArrayRef = Arc::new(StringArray::from_iter_values((0..1_000_000).map(|row| {
        if row == 999_999 {
            "needle".to_owned()
        } else {
            format!("value-{row}")
        }
    })));
    let (_text_file, text_doc) = document(vec![("value", text)]);
    let no_hit = text_doc.prepare_find(0, "absent", &[0]).unwrap();
    group.bench_function("text_no_hit_1m", |b| {
        b.iter(|| black_box(no_hit.next(None, 1).unwrap()))
    });
    let sparse = text_doc.prepare_find(0, "needle", &[0]).unwrap();
    group.bench_function("text_last_hit_1m", |b| {
        b.iter(|| black_box(sparse.next(None, 1).unwrap()))
    });

    let lists: ArrayRef = Arc::new(ListArray::from_iter_primitive::<
        arrow::datatypes::Int32Type,
        _,
        _,
    >((0..100_000).map(|_| Some(vec![Some(1)]))));
    let (_list_file, list_doc) = document(vec![("value", lists)]);
    let nested = list_doc.prepare_find(0, "2", &[0]).unwrap();
    group.bench_function("nested_no_hit_100k", |b| {
        b.iter(|| black_box(nested.next(None, 1).unwrap()))
    });

    let intervals: ArrayRef = Arc::new(IntervalYearMonthArray::from(vec![1; 200_000]));
    let (_interval_file, interval_doc) = document(vec![("value", intervals)]);
    let formatted = interval_doc.prepare_find(0, "absent", &[0]).unwrap();
    group.bench_function("formatted_no_hit_200k", |b| {
        b.iter(|| black_box(formatted.next(None, 1).unwrap()))
    });

    let dictionary: ArrayRef = Arc::new(
        DictionaryArray::<Int32Type>::try_new(
            Int32Array::from_iter_values((0..100_000).chain(std::iter::repeat_n(0, 20_000))),
            Arc::new(StringArray::from_iter_values(
                (0..100_000).map(|row| row.to_string()),
            )),
        )
        .unwrap(),
    );
    let selected: ArrayRef = Arc::new(BooleanArray::from_iter(
        std::iter::repeat_n(false, 100_000).chain(std::iter::repeat_n(true, 20_000)),
    ));
    let (dictionary_file, dictionary_doc) =
        document(vec![("value", dictionary), ("selected", selected)]);
    // Validate the reopened Parquet representation, not just the input fixture.
    let mut reader = ParquetRecordBatchReaderBuilder::try_new(dictionary_file.reopen().unwrap())
        .unwrap()
        .with_batch_size(120_000)
        .build()
        .unwrap();
    let reopened = reader.next().unwrap().unwrap();
    let dictionary = reopened.column(0).as_dictionary::<Int32Type>();
    assert_eq!(dictionary.len(), 120_000);
    assert_eq!(dictionary.values().len(), 100_000);
    assert!(reader.next().is_none());
    drop(reopened);
    drop(reader);
    let filtered = dictionary_doc
        .apply_view(&ViewState {
            filters: vec![Filter::Equals {
                column: 1,
                value: "true".into(),
            }],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(filtered.row_count, 20_000);
    let dictionary_find = dictionary_doc
        .prepare_find(filtered.version, "absent", &[0])
        .unwrap();
    group.bench_function("dictionary_filtered_20k_of_120k_values_100k", |b| {
        b.iter(|| black_box(dictionary_find.next(None, 1).unwrap()))
    });

    let len = 200_000;
    let large_list: ArrayRef = Arc::new(ListArray::new(
        Arc::new(Field::new("item", DataType::Utf8, true)),
        OffsetBuffer::new(vec![0_i32, len].into()),
        Arc::new(StringArray::from_iter_values((0..len).map(|_| "value"))),
        None,
    ));
    let (_large_list_file, large_list_doc) = document(vec![("value", large_list)]);
    let large_list_find = large_list_doc.prepare_find(0, "absent", &[0]).unwrap();
    group.bench_function("one_list_children_200k", |b| {
        b.iter(|| black_box(large_list_find.next(None, 1).unwrap()))
    });

    let rows = 100_000;
    let numbers: ArrayRef = Arc::new(Int32Array::from_iter_values(0..rows));
    let text: ArrayRef = Arc::new(StringArray::from_iter_values((0..rows).map(|_| "value")));
    let mut columns = vec![("number", numbers)];
    columns.extend((0..7).map(|index| {
        (
            Box::leak(format!("text{index}").into_boxed_str()) as &str,
            text.clone(),
        )
    }));
    let (_wide_file, wide_doc) = document(columns);
    let sorted = wide_doc
        .apply_view(&ViewState {
            sort: vec![SortKey {
                column: 0,
                descending: true,
            }],
            ..Default::default()
        })
        .unwrap();
    let wide_columns: Vec<u32> = (0..8).collect();
    let wide_find = wide_doc
        .prepare_find(sorted.version, "absent", &wide_columns)
        .unwrap();
    group.bench_function("sorted_wide_no_hit_100k_8_columns", |b| {
        b.iter(|| black_box(wide_find.next(None, 1).unwrap()))
    });
    group.finish();
}

criterion_group!(benches, find_scans);
criterion_main!(benches);
