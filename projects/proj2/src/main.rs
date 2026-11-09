use std::env;
use std::fs::File;
use std::io::{self, BufReader};

use proj2::{FileReader, Metrics};

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
    let file = File::open(path)?;
    let mut reader = FileReader::new(BufReader::new(file))?;
    let mut metrics = Metrics::new(reader.schema());
    while let Some(batch) = reader.next_batch()? {
        metrics.update(&batch)?;
    }
    print!("{}", metrics.report());
    Ok(())
}
