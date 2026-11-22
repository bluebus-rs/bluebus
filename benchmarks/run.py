#!/usr/bin/env python3
"""Runs every client against bench-peripheral and writes results.json + results.md.

    docker run -d --rm --name bluebus-vhci --privileged bluebus-btvirt -l2
    cargo build --release
    python3 run.py hci1 hci2 --runs 10

The peripheral runs on the first adapter, the clients on the second, one at a
time and interleaved (client A, B, C, A, B, C, ...), so drift affects all.
"""
import argparse
import json
import os
import pathlib
import statistics
import subprocess
import sys
import tempfile
import time

HERE = pathlib.Path(__file__).resolve().parent
REL = HERE / "target" / "release"
PY = HERE / "python" / ".venv" / "bin" / "python"


def clients():
    c = {
        "bluebus": [str(REL / "bench-bluebus")],
        "bluer": [str(REL / "bench-bluer")],
        "btleplug": [str(REL / "bench-btleplug")],
        "bleak (Python)": [str(PY), str(HERE / "python" / "bench_bleak.py")],
    }
    # The same clients with notifications through AcquireNotify (a socket)
    # instead of StartNotify (D-Bus signals); btleplug has no such mode.
    for k in ["bluebus", "bluer", "bleak (Python)"]:
        c[k + " fd"] = c[k] + ["@acquire"]
    if (HERE / "python" / "bench_bluebus.py").exists():
        try:
            subprocess.run([str(PY), "-c", "import bluebus"], check=True, capture_output=True)
            c["bluebus (Python)"] = [str(PY), str(HERE / "python" / "bench_bluebus.py")]
        except subprocess.CalledProcessError:
            pass
    return c


def bus_path(hci, addr):
    return f"/org/bluez/{hci}/dev_{addr.replace(':', '_')}"


def forget(hci, addr):
    subprocess.run(["busctl", "call", "org.bluez", f"/org/bluez/{hci}", "org.bluez.Adapter1",
                    "RemoveDevice", "o", bus_path(hci, addr)], capture_output=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("peripheral_hci")
    ap.add_argument("client_hci")
    ap.add_argument("--runs", type=int, default=10)
    ap.add_argument("--reads", type=int, default=500)
    ap.add_argument("--writes", type=int, default=500)
    ap.add_argument("--notifications", type=int, default=5000)
    ap.add_argument("--only", nargs="*")
    a = ap.parse_args()

    periph = subprocess.Popen([str(REL / "bench-peripheral"), a.peripheral_hci],
                              stdout=subprocess.PIPE, text=True)
    line = periph.stdout.readline().split()
    assert line and line[0] == "READY", line
    addr = line[1]
    results = {}
    try:
        for run in range(a.runs):
            for name, cmd in clients().items():
                if a.only and name not in a.only:
                    continue
                forget(a.client_hci, addr)
                time.sleep(0.5)
                # Started through /usr/bin/time, a small C program: a child forked
                # directly from Python would count Python's memory in its peak RSS.
                # `time` reports the client's peak RSS; wait4 on `time` returns the
                # client's CPU time (RUSAGE_BOTH) with microsecond resolution.
                extra = [x[1:] for x in cmd if x.startswith("@")]
                cmd = [x for x in cmd if not x.startswith("@")]
                full = ["/usr/bin/time", "-f", "RSS_KB %M", *cmd, a.client_hci, str(a.reads),
                        str(a.writes), str(a.notifications), *extra]
                with tempfile.TemporaryFile("w+") as errf:
                    p = subprocess.Popen(full, stdout=subprocess.PIPE, stderr=errf, text=True)
                    out = p.stdout.read()
                    # wait4 reaps the child and returns its own CPU time and peak RSS.
                    _, status, ru = os.wait4(p.pid, 0)
                    errf.seek(0)
                    err = errf.read()
                if status != 0:
                    ru = None
                rec = None
                for l in out.splitlines():
                    if l.startswith("{"):
                        rec = json.loads(l)
                if rec is None or ru is None:
                    print(f"run {run} {name}: FAILED\n{err[-2000:]}", file=sys.stderr)
                    continue
                rec["cpu_ms"] = (ru.ru_utime + ru.ru_stime) * 1e3
                rss = [l.split()[1] for l in err.splitlines() if l.startswith("RSS_KB ")]
                rec["max_rss_mb"] = int(rss[-1]) / 1024
                rec.pop("lib", None)
                results.setdefault(name, []).append(rec)
                print(f"run {run} {name}: read p50 {rec['read_p50_ms']} ms, "
                      f"notify {rec['notify_received']} in {rec['notify_ms']} ms, "
                      f"cpu {rec['cpu_ms']:.0f} ms, rss {rec['max_rss_mb']:.1f} MB", flush=True)
    finally:
        periph.terminate()
    meta = {
        "runs": a.runs, "reads": a.reads, "writes": a.writes, "notifications": a.notifications,
        "bluez": subprocess.run(["bluetoothctl", "--version"], capture_output=True, text=True).stdout.strip(),
        "kernel": os.uname().release,
        "cpu": next((l.split(":", 1)[1].strip() for l in open("/proc/cpuinfo") if l.startswith("model name")), ""),
    }
    (HERE / "results.json").write_text(json.dumps({"meta": meta, "results": results}, indent=1))
    summarize(meta, results)


def summarize(meta, results):
    medians = {n: {k: statistics.median(r[k] for r in rs) for k in rs[0]
                   if isinstance(rs[0][k], (int, float))} for n, rs in results.items()}
    (HERE / "results.md").write_text(table(meta, medians))
    print(table(meta, medians))


def table(meta, m):
    n = meta["notifications"]
    rows = [
        ("Startup (session + adapter ready)", "startup_ms", "{:.1f} ms"),
        ("Connect + GATT services resolved", "connect_ms", "{:.1f} ms"),
        ("Read, p50", "read_p50_ms", "{:.3f} ms"),
        ("Read, p95", "read_p95_ms", "{:.3f} ms"),
        ("Write with response, p50", "write_p50_ms", "{:.3f} ms"),
        ("Write with response, p95", "write_p95_ms", "{:.3f} ms"),
        (f"{n} notifications: received", "notify_received", "{:.0f}"),
        (f"{n} notifications: time", "notify_ms", "{:.1f} ms"),
        ("CPU time: reads + writes", "cpu_rw_ms", "{:.0f} ms"),
        (f"CPU time: {n} notifications", "cpu_notify_ms", "{:.0f} ms"),
        ("CPU time, whole process", "cpu_ms", "{:.0f} ms"),
        ("Peak memory (RSS)", "max_rss_mb", "{:.1f} MB"),
    ]
    names = list(m)
    out = ["| | " + " | ".join(names) + " |", "|---|" + "---:|" * len(names)]
    for label, key, fmt in rows:
        out.append(f"| {label} | " + " | ".join(fmt.format(m[x][key]) for x in names) + " |")
    return "\n".join(out) + "\n"


if __name__ == "__main__":
    if sys.argv[1:] == ["--summarize"]:
        d = json.loads((HERE / "results.json").read_text())
        summarize(d["meta"], d["results"])
    else:
        main()
