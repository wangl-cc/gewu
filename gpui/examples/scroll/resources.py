"""Measure a release scroll example and WindowServer during complete one-second intervals on macOS."""

import plistlib
import re
import statistics
import subprocess
import sys
import threading
import time


def command(*args):
    return subprocess.check_output(args)


def cpu_seconds(value):
    days, _, value = value.rpartition("-")
    total = 0.0
    for part in value.split(":"):
        total = 60 * total + float(part)
    return 86400 * int(days or 0) + total


def snapshot(pids):
    processes = {}
    for line in (
        command("/bin/ps", "-o", "pid=,time=,rss=", "-p", ",".join(pids))
        .decode()
        .splitlines()
    ):
        pid, cpu, rss = line.split()
        processes[pid] = (cpu_seconds(cpu), int(rss) / 1024)

    clients = plistlib.loads(
        command("/usr/sbin/ioreg", "-a", "-r", "-c", "AGXDeviceUserClient")
    )
    gpu = {}
    for client in clients:
        creator = re.match(r"pid (\d+)\b", client.get("IOUserClientCreator", ""))
        if creator and creator[1] in pids:
            values = [
                u["accumulatedGPUTime"]
                for u in client.get("AppUsage", [])
                if "accumulatedGPUTime" in u
            ]
            if values:
                gpu[creator[1]] = gpu.get(creator[1], 0) + sum(values)
    return time.monotonic(), processes, gpu


def main(args):
    ws = command("/usr/bin/pgrep", "-x", "WindowServer").decode().split()[0]
    with subprocess.Popen(args, stdout=subprocess.PIPE, text=True, bufsize=1) as app:
        pids = [str(app.pid), ws]
        phases = []

        def read_output():
            start = None
            seconds = None
            for line in app.stdout:
                now = time.monotonic()
                print(line.rstrip(), flush=True)
                if line.startswith("idle seconds="):
                    seconds = float(line.split("=")[1])
                    start = now
                elif line.startswith("bench: ready") and start is not None:
                    phases.append(("idle", start, now))
                    start = now + 1
                elif (
                    line.startswith("mode=")
                    and start is not None
                    and seconds is not None
                ):
                    phases.append((line.split()[0][5:], start, now))
                    start = now + 1

        reader = threading.Thread(target=read_output)
        reader.start()
        samples = []
        while app.poll() is None:
            try:
                samples.append(snapshot(pids))
            except subprocess.CalledProcessError:
                if app.poll() is None:
                    raise
            time.sleep(1)
        reader.join()
        if app.returncode:
            return app.returncode

    if len(phases) != 4:
        raise ValueError("missing idle or scrolling phase boundaries")
    report(samples, pids, phases)
    return 0


def report(samples, pids, phases):
    for label, start, end in phases:
        within = [
            s
            for s in samples
            if start <= s[0] <= end and all(pid in s[1] for pid in pids)
        ]
        if len(within) < 2:
            raise ValueError(
                f"insufficient samples in {label}; use a duration of at least five seconds"
            )
        a, b = within[0], within[-1]
        elapsed = b[0] - a[0]
        combined = 0.0
        for kind, pid in zip(("app", "WindowServer"), pids):
            cpu = 100 * (b[1][pid][0] - a[1][pid][0]) / elapsed
            combined += cpu
            rss = [s[1][pid][1] for s in within]
            gpu = "unavailable"
            counters = [sample[2].get(pid) for sample in within]
            if None not in counters and counters == sorted(counters):
                gpu = f"{100 * (b[2][pid] - a[2][pid]) / 1e9 / elapsed:.2f}%"
            print(
                f"{label} {kind} CPU={cpu:.2f}% GPU={gpu} RSS_MiB_mean={statistics.mean(rss):.1f} RSS_MiB_peak={max(rss):.1f}"
            )
        print(
            f"{label} combined_CPU={combined:.2f}% samples={len(within)} seconds={elapsed:.2f}"
        )


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
