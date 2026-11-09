use std::env;
use std::fs::File;
use std::io::{self, BufWriter};

use proj2::{Batch, Column, ColumnData, FileWriter};

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let mut args = env::args_os();
    let program = args.next().unwrap_or_default();
    let Some(path) = args.next() else {
        eprintln!("Usage: {} <file.isbd>", program.to_string_lossy());
        std::process::exit(2);
    };
    if args.next().is_some() {
        eprintln!("Usage: {} <file.isbd>", program.to_string_lossy());
        std::process::exit(2);
    }

    let batch = Batch::new(vec![
        Column::new("id", ColumnData::Int64(vec![10, 11, 13, 16, 20, 25])),
        Column::new("active", ColumnData::Boolean(vec![1, 0, 1, 1, 0, 1])),
        Column::new(
            "name",
            ColumnData::Varchar(
                vec!["Alice", "Bob", "Chloë", "", "Eve", "Zoë"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            ),
        ),
        Column::new(
            "city",
            ColumnData::Varchar(
                vec!["London", "München", "Zürich", "Paris", "Boston", "Tokyo"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            ),
        ),
    ])?;
    let file = File::create(path)?;
    let mut writer = FileWriter::new(BufWriter::new(file), &batch.schema())?;
    writer.write_batch(&batch)?;
    writer.finish()?;
    Ok(())
}
