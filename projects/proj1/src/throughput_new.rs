use std::env;
use std::fs::File;
use std::os::fd::AsRawFd;

mod hash;
use hash::sum_block_hash;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if !(2..=3).contains(&args.len()) {
        return Err("Usage: throughput_new filepath [block_size=512]".into());
    }
    let block_size: usize = args.get(2).map_or("512", String::as_str).parse()?;
    if block_size == 0 {
        return Err("Block size must be positive".into());
    }

    let file = File::open(&args[1])?;
    let total_size = usize::try_from(file.metadata()?.len())?;
    let mut result = vec![0u8; 16];
    if total_size > 0 {
        let address = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                total_size,
                libc::PROT_READ,
                libc::MAP_PRIVATE,
                file.as_raw_fd(),
                0,
            )
        };
        if address == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error().into());
        }
        let data = unsafe { std::slice::from_raw_parts(address.cast::<u8>(), total_size) };
        for buffer in data.chunks(block_size) {
            let block_hash = md5::compute(buffer);
            sum_block_hash(&mut result, &block_hash);
        }
        if unsafe { libc::munmap(address, total_size) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }

    print!("{} 0x", args[1]);
    for byte in &result {
        print!("{:02x}", byte);
    }
    println!();
    Ok(())
}
