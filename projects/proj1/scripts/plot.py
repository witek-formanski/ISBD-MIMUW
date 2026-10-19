import argparse
import csv
from collections import defaultdict
import os
from pathlib import Path
import statistics

os.environ.setdefault("MPLCONFIGDIR", "/tmp/isbd-matplotlib")
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

KEYS = ["program", "cache", "size_bytes", "block_size", "k", "version", "threads"]
LABELS = {"warm": "Warm cache", "cold": "Cold Linux cache", "stream": "File larger than RAM"}


def aggregate(paths):
    groups = defaultdict(list)
    for path in paths:
        with Path(path).open() as file:
            for row in csv.DictReader(file):
                for key in ["size_bytes", "block_size", "k", "threads", "repeat", "read_bytes"]:
                    row[key] = int(row[key])
                for key in ["seconds", "user_seconds", "system_seconds", "resident_before"]:
                    row[key] = float(row[key])
                groups[tuple(row[key] for key in KEYS)].append(row)
    summary = []
    for key, rows in sorted(groups.items()):
        best = min(rows, key=lambda row: row["seconds"])
        summary.append(dict(zip(KEYS, key), samples=len(rows),
                            minimum=best["seconds"], median=statistics.median(row["seconds"] for row in rows),
                            maximum=max(row["seconds"] for row in rows),
                            cpu=best["user_seconds"] + best["system_seconds"],
                            system=best["system_seconds"], read_bytes=best["read_bytes"],
                            resident_before=best["resident_before"]))
    return summary


def block_label(value):
    if value >= 1048576:
        return f"{value / 1048576:g} MiB"
    if value >= 1024:
        return f"{value / 1024:g} KiB"
    return f"{value} B"


def line(ax, rows, x, label, metric="minimum", **style):
    rows = sorted(rows, key=lambda row: row[x])
    if not rows:
        return
    xs = [row[x] for row in rows]
    plotted, = ax.plot(xs, [row[metric] for row in rows], marker="o", label=label, **style)
    if metric == "minimum":
        ax.fill_between(xs, [row["minimum"] for row in rows], [row["median"] for row in rows],
                        color=plotted.get_color(), alpha=0.12)


def panels(rows):
    modes = [mode for mode in LABELS if any(row["cache"] == mode for row in rows)]
    fig, axes = plt.subplots(1, len(modes), figsize=(6 * len(modes), 4.5), squeeze=False)
    for ax, mode in zip(axes[0], modes):
        subset = [row for row in rows if row["cache"] == mode]
        sizes = {row["size_bytes"] for row in subset}
        if len(sizes) != 1:
            raise ValueError("Plot one file size per cache mode and program at a time")
        ax.set_title(f"{LABELS[mode]}, {sizes.pop() / 1048576:g} MiB")
        ax.set_ylabel("Time [s]")
        ax.grid(alpha=0.25)
        yield fig, ax, subset


def save(fig, output, name):
    for ax in fig.axes:
        ax.legend(fontsize=8)
    fig.tight_layout()
    fig.savefig(output / f"{name}.png", dpi=160)
    plt.close(fig)


def block_axis(ax, rows):
    blocks = sorted({row["block_size"] for row in rows})
    ax.set_xscale("log", base=2)
    ax.set_xticks(blocks, [block_label(value) for value in blocks], rotation=20)
    ax.set_xlabel("Block size M")


def plot_latency(rows, output):
    for fig, ax, subset in panels(rows):
        line(ax, [row for row in subset if row["version"] == "old"], "block_size", "Original", color="black", linestyle="--")
        for threads in sorted({row["threads"] for row in subset if row["version"] == "new"}):
            line(ax, [row for row in subset if row["version"] == "new" and row["threads"] == threads],
                 "block_size", f"New, {threads} readers")
        block_axis(ax, subset)
    save(fig, output, "latency_blocks")
    for fig, ax, subset in panels(rows):
        for index, block in enumerate(sorted({row["block_size"] for row in subset})):
            points = [row for row in subset if row["block_size"] == block and row["version"] == "new"]
            line(ax, points, "threads", f"New, {block_label(block)}", color=f"C{index}")
            old = next(row for row in subset if row["block_size"] == block and row["version"] == "old")
            ax.axhline(old["minimum"], color=f"C{index}", linestyle="--", label=f"Original, {block_label(block)}")
        ax.set_xticks(sorted({row["threads"] for row in subset}))
        ax.set_xlabel("Reader threads (plus one hash thread)")
    save(fig, output, "latency_threads")


def plot_throughput(rows, output):
    for metric, name in [("minimum", "throughput_time"), ("cpu", "throughput_cpu")]:
        for fig, ax, subset in panels(rows):
            for version, label in [("old", "Original (pread)"), ("new", "New (mmap)")]:
                line(ax, [row for row in subset if row["version"] == version], "block_size", label, metric=metric)
            block_axis(ax, subset)
            if metric == "cpu":
                ax.set_ylabel("CPU time: user + system [s]")
        save(fig, output, name)


def plot_pipeline(rows, output):
    k_rows = [row for row in rows if row["block_size"] == 1048576]
    if k_rows:
        for fig, ax, subset in panels(k_rows):
            for version, label in [("old", "Original"), ("new", "New (pipeline)")]:
                line(ax, [row for row in subset if row["version"] == version], "k", label)
            ks = sorted({row["k"] for row in subset})
            ax.set_xscale("log")
            ax.set_xticks(ks, [str(k) for k in ks], rotation=30)
            ax.set_xlabel("K (M = 1 MiB)")
        save(fig, output, "pipeline_k")
    m_rows = [row for row in rows if row["cache"] != "stream"]
    if len({row["block_size"] for row in m_rows}) > 1:
        for fig, ax, subset in panels(m_rows):
            for index, k in enumerate(sorted({row["k"] for row in subset})):
                points = [row for row in subset if row["k"] == k]
                if len({row["block_size"] for row in points}) < 2:
                    continue
                for version in ["old", "new"]:
                    line(ax, [row for row in points if row["version"] == version], "block_size", f"{version}, K={k}",
                         color=f"C{index}", linestyle="--" if version == "old" else "-")
            block_axis(ax, subset)
        save(fig, output, "pipeline_blocks")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("csv", nargs="+")
    parser.add_argument("--output", default="plots")
    args = parser.parse_args()
    output = Path(args.output)
    output.mkdir(parents=True, exist_ok=True)
    rows = aggregate(args.csv)
    with (output / "summary.csv").open("w", newline="") as file:
        writer = csv.DictWriter(file, fieldnames=list(rows[0]), lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)
    for name, plot in [("latency", plot_latency), ("throughput", plot_throughput), ("pipeline", plot_pipeline)]:
        subset = [row for row in rows if row["program"] == name]
        if subset:
            plot(subset, output)


if __name__ == "__main__":
    main()
