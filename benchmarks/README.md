# Benchmarks

bluebus against [bluer](https://crates.io/crates/bluer),
[btleplug](https://crates.io/crates/btleplug) and Python's
[bleak](https://pypi.org/project/bleak/), all talking to the same GATT
server through the real BlueZ daemon, on two virtual controllers made by
BlueZ' `btvirt` (no radio, so the numbers show what the *library* and BlueZ
cost, not the air link).

## What is measured

`peripheral/` is one GATT server for every client (built on bluebus, the
same for all): a 20-byte readable characteristic, a writable one and a
notifying one that sends N notifications of 20 bytes when asked.

Each client (`clients/*`, `python/bench_bleak.py`) runs the same scenario
with its library's normal API, on a multi-threaded tokio runtime for the
Rust ones:

1. **startup**: system bus connection and adapter ready (Python: includes
   `import bleak`);
2. **discovery** of the server by name (not compared: on virtual
   controllers it is dominated by the kernel's scan timing, ~10 s or ~0 s,
   for every library);
3. **connect** and resolve the GATT services;
4. **500 reads** and **500 writes with response**: latency p50 / p95 and
   the CPU time spent;
5. **5000 notifications**: received, time from request to the last one,
   CPU time. Twice: through `StartNotify` (each value a D-Bus signal) and,
   where the library supports it, through `AcquireNotify` (a socket;
   columns marked *fd*). btleplug has no AcquireNotify mode;
6. **peak memory** (RSS) and total CPU time of the process.

`run.py` runs the clients one at a time, interleaved (A, B, C, A, B, C …),
forgets the device between runs and reports medians. CPU time is read from
`/proc/self/task/*/schedstat` (Rust) and `time.process_time()` (Python),
memory with `/usr/bin/time`.

## Run

```sh
docker build -t bluebus-btvirt ../tools/btvirt
docker run -d --rm --name bluebus-vhci --privileged bluebus-btvirt -l2   # adds two controllers
bluetoothctl list                                                        # find their hciN names
cargo build --release                       # needs libdbus-1-dev for bluer and btleplug
uv venv python/.venv && uv pip install --python python/.venv/bin/python --require-hashes -r python/requirements.txt
python3 run.py hci1 hci2 --runs 10          # peripheral on hci1, clients on hci2
docker stop bluebus-vhci
```

Results: `results.md` (table) and `results.json` (every run);
`python3 run.py --summarize` rebuilds the table from `results.json`.
`bluebus (Python)` needs the bindings installed in `python/.venv`
(`maturin build --release` in `../python`, then `uv pip install` the wheel).

## Results

10 runs, medians. Intel Core Ultra 7 255HX, Linux 7.0, BlueZ 5.72, two
btvirt controllers; bluebus 0.1.11+ (main), bluer 0.17.4, btleplug 0.13.5,
bleak 3.1.0 (dbus-fast 5.2.0, Python 3.13). *fd* = notifications through
`AcquireNotify`.

| | bluebus | bluer | btleplug | bleak (Python) | bluebus fd | bluer fd | bleak (Python) fd | bluebus (Python) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Startup (session + adapter ready) | 3.4 ms | 3.3 ms | 3.5 ms | 30.1 ms | 3.9 ms | 2.7 ms | 29.1 ms | 31.7 ms |
| Connect + GATT services resolved | 26.0 ms | 30.1 ms | 30.8 ms | 36.5 ms | 24.2 ms | 30.5 ms | 28.6 ms | 23.9 ms |
| Read, p50 | 0.280 ms | 0.293 ms | 0.278 ms | 0.301 ms | 0.309 ms | 0.287 ms | 0.299 ms | 0.420 ms |
| Read, p95 | 0.866 ms | 0.839 ms | 0.698 ms | 0.885 ms | 1.005 ms | 0.905 ms | 0.956 ms | 1.085 ms |
| Write with response, p50 | 0.105 ms | 0.105 ms | 0.107 ms | 0.121 ms | 0.111 ms | 0.107 ms | 0.137 ms | 0.164 ms |
| Write with response, p95 | 0.586 ms | 0.452 ms | 0.670 ms | 0.583 ms | 0.527 ms | 0.430 ms | 0.721 ms | 0.829 ms |
| 5000 notifications: received | 5000 | 5000 | 5000 | 5001 | 5000 | 5000 | 5000 | 5000 |
| 5000 notifications: time | 424.1 ms | 414.4 ms | 428.5 ms | 419.2 ms | 95.7 ms | 96.6 ms | 86.0 ms | 432.7 ms |
| CPU time: reads + writes | 28 ms | 31 ms | 32 ms | 46 ms | 29 ms | 34 ms | 52 ms | 93 ms |
| CPU time: 5000 notifications | 66 ms | 58 ms | 63 ms | 35 ms | 7 ms | 8 ms | 13 ms | 243 ms |
| CPU time, whole process | 113 ms | 110 ms | 116 ms | 162 ms | 53 ms | 61 ms | 139 ms | 398 ms |
| Peak memory (RSS) | 5.4 MB | 5.9 MB | 7.7 MB | 27.9 MB | 5.4 MB | 5.8 MB | 27.9 MB | 29.1 MB |

What it shows:

- Reads, writes and connects cost the same with every library: BlueZ and
  the D-Bus round trip dominate.
- Notifications through `AcquireNotify` (bluebus
  `characteristic_notifications_acquired`, bluer `notify_io`, bleak
  `use_start_notify=False`) are 4–5× faster and need 5–10× less CPU than
  through D-Bus signals; btleplug cannot use it.
- Through D-Bus signals, bleak (dbus-fast, Cython) uses the least CPU;
  bluebus, bluer and btleplug are within 15 % of each other.
- The bluebus Python bindings are far from bleak for notifications (243 ms
  vs 35 ms CPU): each value crosses into Python separately. To be fixed.

