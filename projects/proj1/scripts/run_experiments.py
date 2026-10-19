import argparse
from pathlib import Path
import subprocess
import sys

from build import ROOT, build


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("suite", choices=["main", "stream", "probes"])
    parser.add_argument("--output-dir", default="results")
    args = parser.parse_args()
    build()
    common = [sys.executable, str(Path(__file__).with_name("measure.py")), "--skip-build"]
    suites = {
        "main": [
            "latency --file data/latency128.bin --size-mib 128 --blocks 512 4096 16384 65536 --threads 1 2 4 8 --repeats 5 --output results/latency.csv",
            "throughput --file data/sequential.bin --size-mib 512 --blocks 512 1024 4096 16384 65536 262144 1048576 --repeats 5 --output results/throughput.csv",
            "pipeline --file data/sequential.bin --size-mib 512 --blocks 1048576 --ks 1 100 500 1000 2000 4000 16000 --repeats 5 --output results/pipeline_k.csv",
            "pipeline --file data/sequential.bin --size-mib 512 --blocks 65536 262144 4194304 --ks 1 1000 16000 --repeats 5 --output results/pipeline_m.csv",
        ],
        "stream": [
            "pipeline --file data/stream.bin --size-mib 9216 --blocks 1048576 --ks 1 500 1000 4000 --cache stream --repeats 3 --output results/pipeline_stream.csv",
        ],
        "probes": [
            "latency --file data/latency128.bin --size-mib 128 --blocks 512 4096 --threads 1 2 4 8 --repeats 3 --probe --output results/latency_probe.csv",
            "pipeline --file data/sequential.bin --size-mib 512 --blocks 1048576 --ks 1 1000 4000 --repeats 3 --probe --output results/pipeline_probe.csv",
            "pipeline --file data/stream.bin --size-mib 9216 --blocks 1048576 --ks 1 4000 --cache stream --repeats 3 --probe --output results/pipeline_stream_probe.csv",
        ],
    }
    for experiment in suites[args.suite]:
        parameters = experiment.split()
        output_index = parameters.index("--output") + 1
        parameters[output_index] = str(Path(args.output_dir) / Path(parameters[output_index]).name)
        subprocess.run(common + parameters, cwd=ROOT, check=True)


if __name__ == "__main__":
    main()
