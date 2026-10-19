import argparse
import csv
import ctypes
from datetime import datetime, timezone
import hashlib
import itertools
import json
import mmap
import os
from pathlib import Path
import platform
import random
import resource
import subprocess
import time

from build import ROOT, BASELINE, build, command

MIB = 1024 * 1024


def prepare_file(path, size_mib):
    size = size_mib * MIB
    if path.exists():
        if path.stat().st_size != size:
            raise ValueError(f"Existing file has a different size: {path}")
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    rng = random.Random(1234)
    print(f"Creating {path}: {size_mib} MiB", flush=True)
    with path.open("xb") as output:
        for _ in range(size_mib):
            output.write(rng.randbytes(MIB))
        output.flush()
        os.fsync(output.fileno())


def resident_fraction(path):
    with path.open("rb") as file, mmap.mmap(file.fileno(), 0, access=mmap.ACCESS_COPY) as mapping:
        address = ctypes.addressof(ctypes.c_char.from_buffer(mapping))
        pages = (len(mapping) + mmap.PAGESIZE - 1) // mmap.PAGESIZE
        vector = (ctypes.c_ubyte * pages)()
        libc = ctypes.CDLL(None, use_errno=True)
        result = libc.mincore(ctypes.c_void_p(address), ctypes.c_size_t(len(mapping)), vector)
        if result != 0:
            raise OSError(ctypes.get_errno(), "mincore failed")
        return sum(value & 1 for value in vector) / pages


def prepare_cache(path, mode):
    with path.open("rb", buffering=0) as file:
        if mode == "warm":
            buffer = bytearray(8 * MIB)
            while file.readinto(buffer):
                pass
        else:
            os.posix_fadvise(file.fileno(), 0, 0, os.POSIX_FADV_DONTNEED)
    fraction = resident_fraction(path)
    if mode == "warm" and fraction < 0.99:
        raise RuntimeError(f"File does not fit in cache: {fraction:.1%} resident")
    if mode != "warm" and fraction > 0.01:
        raise RuntimeError(f"Cache eviction failed: {fraction:.1%} resident")
    return fraction


def timed_run(args):
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    start = time.perf_counter_ns()
    completed = subprocess.run(args, capture_output=True, text=True, check=True)
    seconds = (time.perf_counter_ns() - start) / 1e9
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    output = completed.stdout.split()
    if not output or len(output[-1]) != 34 or not output[-1].startswith("0x"):
        raise RuntimeError(f"Missing hash: {completed.stdout} {completed.stderr}")
    stages = [line.split()[1:] for line in completed.stderr.splitlines() if line.startswith("stages ")]
    return {
        "seconds": seconds,
        "user_seconds": after.ru_utime - before.ru_utime,
        "system_seconds": after.ru_stime - before.ru_stime,
        "read_bytes": (after.ru_inblock - before.ru_inblock) * 512,
        "major_faults": after.ru_majflt - before.ru_majflt,
        "minor_faults": after.ru_minflt - before.ru_minflt,
        "hash": output[-1],
        "read_calls": sum(int(stage[0]) for stage in stages),
        "read_seconds": sum(float(stage[1]) for stage in stages),
        "compute_seconds": sum(float(stage[2]) for stage in stages),
    }


def metadata(args):
    return {
        "date": datetime.now(timezone.utc).isoformat(),
        "arguments": vars(args),
        "kernel": platform.platform(),
        "cpu": subprocess.check_output(["lscpu"], text=True),
        "memory": Path("/proc/meminfo").read_text(),
        "rust": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "affinity": sorted(os.sched_getaffinity(0)),
        "sources": {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
                    for path in (ROOT / "src").glob("*.rs")},
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("program", choices=["latency", "throughput", "pipeline"])
    parser.add_argument("--file", required=True)
    parser.add_argument("--size-mib", type=int, required=True)
    parser.add_argument("--blocks", type=int, nargs="+")
    parser.add_argument("--threads", type=int, nargs="+", default=[1, 2, 4, 8])
    parser.add_argument("--ks", type=int, nargs="+", default=[1, 1000, 4000])
    parser.add_argument("--cache", choices=["warm", "cold", "stream"], nargs="+", default=["warm", "cold"])
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--output", required=True)
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--probe", action="store_true", help="Measure individual read/compute stages in separate diagnostic binaries")
    args = parser.parse_args()
    if args.blocks is None:
        args.blocks = [65536, 262144, 1048576] if args.program == "pipeline" else [512, 4096, 65536, 1048576]
    if args.probe and args.program == "throughput":
        parser.error("Stage probes are available for latency and pipeline")
    if min([args.size_mib, args.repeats, *args.blocks, *args.threads, *args.ks]) <= 0:
        parser.error("Sizes, counts and k must be positive")
    output = Path(args.output)
    if output.exists():
        parser.error(f"Refusing to overwrite {output}")
    if not args.skip_build:
        build()
    path = Path(args.file).resolve()
    if "stream" in args.cache:
        total_memory = int(Path("/proc/meminfo").read_text().splitlines()[0].split()[1]) * 1024
        if args.size_mib * MIB <= total_memory:
            parser.error("The stream file must be larger than total RAM")
    prepare_file(path, args.size_mib)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.with_suffix(".json").write_text(json.dumps(metadata(args), indent=2))
    cases = []
    for cache, block_size in itertools.product(args.cache, args.blocks):
        for k in (args.ks if args.program == "pipeline" else [1]):
            cases.append((cache, block_size, k, "old", 1))
            if args.probe and args.program == "pipeline":
                continue
            for threads in (args.threads if args.program == "latency" else [1]):
                cases.append((cache, block_size, k, "new", threads))
    fields = ["program", "version", "binary", "cache", "size_bytes", "block_size", "k", "threads", "repeat", "resident_before", "seconds", "user_seconds", "system_seconds", "read_bytes", "major_faults", "minor_faults", "hash", "read_calls", "read_seconds", "compute_seconds"]
    expected = {}
    rng = random.Random(2026)
    with output.open("x", newline="") as file:
        writer = csv.DictWriter(file, fieldnames=fields, lineterminator="\n")
        writer.writeheader()
        for repeat in range(1, args.repeats + 1):
            rng.shuffle(cases)
            for cache, block_size, k, version, threads in cases:
                fraction = prepare_cache(path, cache)
                cmd = command(args.program, version, path, block_size, k, threads)
                if args.probe:
                    suffix = "_new_probe" if version == "new" else "_probe"
                    cmd = [str(BASELINE / f"{args.program}{suffix}"), str(path), str(block_size)]
                    if args.program == "pipeline":
                        cmd.append(str(k))
                    if args.program == "latency" and version == "new":
                        cmd.append(str(threads))
                measurements = timed_run(cmd)
                key = (block_size, k)
                expected.setdefault(key, measurements["hash"])
                if expected[key] != measurements["hash"]:
                    raise RuntimeError(f"Hash mismatch: {cmd}")
                writer.writerow(dict(program=args.program, version=version, binary=Path(cmd[0]).name,
                                     cache=cache, size_bytes=path.stat().st_size, block_size=block_size,
                                     k=k, threads=threads, repeat=repeat, resident_before=fraction,
                                     **measurements))
                file.flush()
                print(f"{repeat}/{args.repeats} {cache} {version} M={block_size} K={k} threads={threads}: {measurements['seconds']:.4f} s", flush=True)


if __name__ == "__main__":
    main()
