import hashlib
from pathlib import Path
import random
import subprocess
import tempfile

from build import BIN, BASELINE, build, command


def reference(data, block_size, k):
    result = 0
    for offset in range(0, len(data), block_size):
        block = data[offset:offset + block_size]
        for _ in range(k):
            block = hashlib.md5(block).digest()
        result ^= int.from_bytes(block, "big")
    return f"0x{result:032x}"


def run(args):
    return subprocess.check_output(args, text=True).split()[-1]


def verify():
    count = 0
    rng = random.Random(1234)
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "input.bin"
        for size in [0, 1, 511, 512, 513, 8193, 1048593]:
            data = rng.randbytes(size)
            path.write_bytes(data)
            for block_size in [512, 4096, 1048576]:
                for name in ["latency", "throughput", "pipeline"]:
                    for k in ([1, 7] if name == "pipeline" else [1]):
                        expected = reference(data, block_size, k)
                        for version in ["old", "new"]:
                            for threads in ([1, 4] if name == "latency" and version == "new" else [1]):
                                args = command(name, version, path, block_size, k, threads)
                                assert run(args) == expected, args
                                count += 1
            for name, block_size, k in [("latency", 512, 1), ("throughput", 512, 1), ("pipeline", 1048576, 1000)]:
                expected = reference(data, block_size, k)
                assert run([str(BIN / name), str(path)]) == expected
                assert run([str(BASELINE / f"{name}_baseline"), str(path)]) == expected
                assert run([str(BIN / f"{name}_new"), str(path)]) == expected
                count += 3
        for name in ["latency", "throughput", "pipeline"]:
            binary = str(BIN / f"{name}_new")
            for args in [[], [str(path), "0"], [str(path), "invalid"], [str(path) + ".missing"]]:
                assert subprocess.run([binary, *args], capture_output=True).returncode != 0
                count += 1
        for name in ["latency", "pipeline"]:
            assert subprocess.run([str(BIN / f"{name}_new"), str(path), "512", "0"], capture_output=True).returncode != 0
            count += 1
    print(f"Passed {count} checks, including independent hashlib results and partial blocks.")


if __name__ == "__main__":
    build()
    verify()
