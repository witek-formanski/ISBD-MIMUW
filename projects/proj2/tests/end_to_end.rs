use std::io::{Cursor, ErrorKind};

use proj2::{
    BATCH_SIZE, Batch, CellRef, Column, ColumnData, DataType, Field, FileReader, FileWriter,
    Metrics,
};

fn batch(start: usize, count: usize) -> Batch {
    let numbers = (start..start + count)
        .map(|index| match index % 4 {
            0 => i64::MIN,
            1 => i64::MAX,
            2 => -(index as i64),
            _ => index as i64,
        })
        .collect();
    let booleans = (start..start + count)
        .map(|index| (index % 3 == 0) as u8)
        .collect();
    let first = (start..start + count)
        .map(|index| match index % 3 {
            0 => String::new(),
            1 => "naïve\0🙂".repeat(20),
            _ => format!("row-{index}"),
        })
        .collect();
    let second = (start..start + count)
        .map(|index| format!(" + {index}"))
        .collect();
    Batch::new(vec![
        Column::new("number", ColumnData::Int64(numbers)),
        Column::new("flag", ColumnData::Boolean(booleans)),
        Column::new("S1", ColumnData::Varchar(first)),
        Column::new("S2", ColumnData::Varchar(second)),
    ])
    .unwrap()
}

#[test]
fn round_trip_multiple_batches_and_row_access() {
    let original = vec![batch(0, BATCH_SIZE), batch(BATCH_SIZE, 17)];
    let mut bytes = Vec::new();
    let mut writer = FileWriter::new(&mut bytes, &original[0].schema()).unwrap();
    for item in &original {
        writer.write_batch(item).unwrap();
    }
    writer.finish().unwrap();

    assert_eq!(&bytes[..4], b"ISBD");

    let mut reader = FileReader::new(Cursor::new(bytes)).unwrap();
    assert_eq!(reader.schema(), original[0].schema());
    for expected in &original {
        assert_eq!(reader.next_batch().unwrap().unwrap(), *expected);
    }
    assert!(reader.next_batch().unwrap().is_none());
    assert!(reader.next_batch().unwrap().is_none());
    assert_eq!(
        original[0].cell_named("number", 0),
        Some(CellRef::Int64(i64::MIN))
    );
    assert_eq!(
        original[0].cell_named("flag", 0),
        Some(CellRef::Boolean(true))
    );
    assert_eq!(
        original[0].cell_named("S1", 1),
        Some(CellRef::Varchar(&"naïve\0🙂".repeat(20)))
    );
    assert_eq!(original[0].cell_named("S1", BATCH_SIZE), None);
}

#[test]
fn varchar_concatenation_creates_column() {
    let mut data = batch(0, 3);
    data.concat_varchar("S3", "S1", "S2").unwrap();
    assert_eq!(data.cell_named("S3", 0), Some(CellRef::Varchar(" + 0")));
    assert_eq!(
        data.cell_named("S3", 2),
        Some(CellRef::Varchar("row-2 + 2"))
    );
    assert_eq!(data.schema().last().unwrap().data_type, DataType::Varchar);
}

#[test]
fn metrics_handle_extreme_integers_unicode_and_entropy_limits() {
    let data = Batch::new(vec![
        Column::new("n", ColumnData::Int64(vec![i64::MIN, i64::MAX])),
        Column::new("b", ColumnData::Boolean(vec![1, 1])),
        Column::new("s", ColumnData::Varchar(vec!["Aé🙂".into(), "B\0".into()])),
    ])
    .unwrap();
    let mut metrics = Metrics::new(&data.schema());
    metrics.update(&data).unwrap();
    assert_eq!(metrics.row_count(), 2);
    assert_eq!(metrics.int64_mean(0), Some(-0.5));
    assert_eq!(metrics.boolean_entropy(1), Some(0.0));
    assert_eq!(metrics.ascii_count(2), Some(3));
}

#[test]
fn empty_file_and_zero_column_batch() {
    let schema = [Field {
        name: "n".into(),
        data_type: DataType::Int64,
    }];
    let mut bytes = Vec::new();
    FileWriter::new(&mut bytes, &schema)
        .unwrap()
        .finish()
        .unwrap();
    let mut reader = FileReader::new(Cursor::new(bytes)).unwrap();
    assert!(reader.next_batch().unwrap().is_none());
    assert!(Metrics::new(&schema).report().contains("no data"));

    let empty_schema: [Field; 0] = [];
    let no_columns = Batch::with_row_count(vec![], 3).unwrap();
    let mut bytes = Vec::new();
    let mut writer = FileWriter::new(&mut bytes, &empty_schema).unwrap();
    writer.write_batch(&no_columns).unwrap();
    writer.finish().unwrap();
    let mut reader = FileReader::new(Cursor::new(bytes)).unwrap();
    assert_eq!(reader.next_batch().unwrap().unwrap(), no_columns);
    assert!(reader.next_batch().unwrap().is_none());
}

#[test]
fn rejects_corrupt_or_truncated_files() {
    let data = Batch::new(vec![Column::new("b", ColumnData::Boolean(vec![1]))]).unwrap();
    let mut bytes = Vec::new();
    let mut writer = FileWriter::new(&mut bytes, &data.schema()).unwrap();
    writer.write_batch(&data).unwrap();
    writer.finish().unwrap();

    let mut bad_magic = bytes.clone();
    bad_magic[0] = 0;
    assert_eq!(
        FileReader::new(Cursor::new(bad_magic))
            .err()
            .unwrap()
            .kind(),
        ErrorKind::InvalidData
    );

    let mut bad_padding = bytes.clone();
    *bad_padding.last_mut().unwrap() = 0x81;
    let mut reader = FileReader::new(Cursor::new(bad_padding)).unwrap();
    assert_eq!(
        reader.next_batch().unwrap_err().kind(),
        ErrorKind::InvalidData
    );

    bytes.pop();
    let mut reader = FileReader::new(Cursor::new(bytes)).unwrap();
    assert_eq!(
        reader.next_batch().unwrap_err().kind(),
        ErrorKind::UnexpectedEof
    );
}

#[test]
fn shipped_sample_has_required_columns_and_expected_metrics() {
    let sample = include_bytes!("../sample.isbd");
    assert_eq!(&sample[..4], b"ISBD");
    let mut reader = FileReader::new(Cursor::new(sample)).unwrap();
    assert_eq!(reader.schema().len(), 4);
    assert_eq!(
        reader
            .schema()
            .iter()
            .map(|field| field.data_type)
            .collect::<Vec<_>>(),
        [
            DataType::Int64,
            DataType::Boolean,
            DataType::Varchar,
            DataType::Varchar
        ]
    );
    let mut metrics = Metrics::new(reader.schema());
    while let Some(batch) = reader.next_batch().unwrap() {
        metrics.update(&batch).unwrap();
    }
    assert_eq!(metrics.row_count(), 6);
    assert_eq!(metrics.int64_mean(0), Some(95.0 / 6.0));
    assert!((metrics.boolean_entropy(1).unwrap() - 0.918295834).abs() < 1e-8);
    assert_eq!(metrics.ascii_count(2), Some(17));
    assert_eq!(metrics.ascii_count(3), Some(33));
}
