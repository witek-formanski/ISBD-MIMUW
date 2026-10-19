use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use std::env;
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::sync::mpsc::sync_channel;
use std::thread;

mod hash;
use hash::sum_block_hash;

struct RandomOrderIterator {
    elements: Vec<u64>,
    rng: StdRng,
}

impl RandomOrderIterator {
    fn with_seed(size: u64, seed: u64) -> Self {
        Self {
            elements: (0..size).collect(),
            rng: StdRng::seed_from_u64(seed),
        }
    }
}

impl Iterator for RandomOrderIterator {
    type Item = u64;

    fn next(&mut self) -> Option<Self::Item> {
        if self.elements.is_empty() {
            None
        } else {
            let rand_idx = self.rng.random_range(0..self.elements.len());
            Some(self.elements.swap_remove(rand_idx))
        }
    }
}

const RAND_SEED: u64 = 1234;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if !(2..=4).contains(&args.len()) {
        return Err("Usage: latency_new filepath [block_size=512] [threads=4]".into());
    }
    let block_size: u64 = args.get(2).map_or("512", String::as_str).parse()?;
    let threads: usize = args.get(3).map_or("4", String::as_str).parse()?;
    if block_size == 0 || threads == 0 {
        return Err("Block size and thread count must be positive".into());
    }

    let file = File::open(&args[1])?;
    let total_size = file.metadata()?.len();
    let num_blocks = total_size.div_ceil(block_size);
    let offsets: Vec<u64> = RandomOrderIterator::with_seed(num_blocks, RAND_SEED)
        .map(|block_idx| block_idx * block_size)
        .collect();
    let mut result = vec![0u8; 16];
    let (sender, receiver) = sync_channel(256);

    thread::scope(|scope| -> std::io::Result<()> {
        let mut readers = Vec::new();
        for part in offsets.chunks(offsets.len().div_ceil(threads).max(1)) {
            let sender = sender.clone();
            let file = &file;
            readers.push(scope.spawn(move || -> std::io::Result<()> {
                for &offset in part {
                    let bytes_to_read = block_size.min(total_size - offset) as usize;
                    let mut buffer = vec![0u8; bytes_to_read];
                    file.read_exact_at(&mut buffer, offset)?;
                    sender.send(buffer).unwrap();
                }
                Ok(())
            }));
        }
        drop(sender);
        for buffer in receiver {
            let block_hash = md5::compute(&buffer);
            sum_block_hash(&mut result, &block_hash);
        }
        for reader in readers {
            reader.join().unwrap()?;
        }
        Ok(())
    })?;

    print!("{} 0x", args[1]);
    for byte in &result {
        print!("{:02x}", byte);
    }
    println!();
    Ok(())
}
