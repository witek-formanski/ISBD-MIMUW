use std::env;
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::sync::mpsc::sync_channel;
use std::thread;

mod hash;
use hash::{stretch_md5, sum_block_hash};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if !(2..=4).contains(&args.len()) {
        return Err("Usage: pipeline_new filepath [block_size=1048576] [k=1000]".into());
    }
    let block_size: u64 = args.get(2).map_or("1048576", String::as_str).parse()?;
    let k: u64 = args.get(3).map_or("1000", String::as_str).parse()?;
    if block_size == 0 || k == 0 {
        return Err("Block size and k must be positive".into());
    }

    let file = File::open(&args[1])?;
    let total_size = file.metadata()?.len();
    let num_blocks = total_size.div_ceil(block_size);
    let mut result = vec![0u8; 16];
    let (sender, receiver) = sync_channel(2);

    thread::scope(|scope| -> std::io::Result<()> {
        let reader = scope.spawn(move || -> std::io::Result<()> {
            for block_idx in 0..num_blocks {
                let offset = block_idx * block_size;
                let bytes_to_read = block_size.min(total_size - offset) as usize;
                let mut buffer = vec![0u8; bytes_to_read];
                file.read_exact_at(&mut buffer, offset)?;
                sender.send(buffer).unwrap();
            }
            Ok(())
        });
        for buffer in receiver {
            let block_hash = stretch_md5(&buffer, k);
            sum_block_hash(&mut result, &block_hash);
        }
        reader.join().unwrap()
    })?;

    print!("{} 0x", args[1]);
    for byte in &result {
        print!("{:02x}", byte);
    }
    println!();
    Ok(())
}
