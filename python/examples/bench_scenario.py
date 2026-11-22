"""Benchmark scenario for the Python bindings (one JSON line on stdout).

    python bench_scenario.py <hciN> <reads> <writes> <notifications> [acquire]

Finds the device named ``bench-periph``, connects, reads characteristic
``…be01`` (20 bytes) ``reads`` times, writes ``…be02`` with response
``writes`` times, subscribes to ``…be03`` and writes the u32 (LE)
``notifications`` to it so the peripheral sends that many notifications,
counts them (3 s idle timeout) and disconnects.

``acquire`` (AcquireWrite/AcquireNotify sockets) is not offered by
``bluebus.ble``; it is accepted for command-line compatibility with the
other scenarios and ignored (a note goes to stderr).
"""

import time

T0 = time.perf_counter()
CPU0 = time.process_time()

import asyncio  # noqa: E402
import json  # noqa: E402
import struct  # noqa: E402
import sys  # noqa: E402

import bluebus  # noqa: E402

NAME = "bench-periph"
READ = "b1eb0000-0000-4000-8000-00000000be01"
WRITE = "b1eb0000-0000-4000-8000-00000000be02"
NOTIFY = "b1eb0000-0000-4000-8000-00000000be03"
IDLE_TIMEOUT = 3.0


def ms(seconds):
    return round(seconds * 1000.0, 3)


def percentile(samples, p):
    if not samples:
        return None
    ordered = sorted(samples)
    k = (len(ordered) - 1) * p / 100.0
    lo = int(k)
    hi = min(lo + 1, len(ordered) - 1)
    return ordered[lo] + (ordered[hi] - ordered[lo]) * (k - lo)


async def run(hci, reads, writes, notifications):
    result = {"lib": "bluebus-py"}
    session = await bluebus.Session.new()
    adapter = await session.adapter(hci)
    await adapter.power_on()
    result["startup_ms"] = ms(time.perf_counter() - T0)

    t = time.perf_counter()
    device = await adapter.find_device(name=NAME, transport="le", timeout=30.0)
    result["discover_ms"] = ms(time.perf_counter() - t)

    t = time.perf_counter()
    await device.connect(timeout=30.0)
    result["connect_ms"] = ms(time.perf_counter() - t)

    cpu = time.process_time()
    read_times = []
    for _ in range(reads):
        t = time.perf_counter()
        value = await device.read(READ)
        read_times.append(time.perf_counter() - t)
        if len(value) != 20:
            raise RuntimeError(f"read {len(value)} bytes, expected 20")
    payload = bytes(range(20))
    write_times = []
    for _ in range(writes):
        t = time.perf_counter()
        await device.write(WRITE, payload, response=True)
        write_times.append(time.perf_counter() - t)
    result["cpu_rw_ms"] = ms(time.process_time() - cpu)
    for key, samples in (("read", read_times), ("write", write_times)):
        p50, p95 = percentile(samples, 50), percentile(samples, 95)
        result[f"{key}_p50_ms"] = ms(p50) if p50 is not None else None
        result[f"{key}_p95_ms"] = ms(p95) if p95 is not None else None

    cpu = time.process_time()
    received = 0
    notify_seconds = 0.0
    async with device.notifications(NOTIFY) as values:
        t = time.perf_counter()
        await device.write(NOTIFY, struct.pack("<I", notifications), response=True)
        last = t
        while received < notifications:
            try:
                await asyncio.wait_for(values.__anext__(), IDLE_TIMEOUT)
            except (asyncio.TimeoutError, StopAsyncIteration):
                break
            received += 1
            last = time.perf_counter()
        notify_seconds = last - t
    result["cpu_notify_ms"] = ms(time.process_time() - cpu)
    result["notify_received"] = received
    result["notify_ms"] = ms(notify_seconds)

    t = time.perf_counter()
    await device.disconnect()
    result["disconnect_ms"] = ms(time.perf_counter() - t)

    order = [
        "lib", "startup_ms", "discover_ms", "connect_ms", "read_p50_ms", "read_p95_ms",
        "write_p50_ms", "write_p95_ms", "notify_received", "notify_ms", "disconnect_ms",
        "cpu_rw_ms", "cpu_notify_ms",
    ]
    return {k: result.get(k) for k in order}


def main(argv):
    if len(argv) < 5:
        print(__doc__.strip().splitlines()[2].strip(), file=sys.stderr)
        sys.exit(2)
    hci = argv[1]
    reads, writes, notifications = (int(a) for a in argv[2:5])
    if len(argv) > 5 and argv[5] == "acquire":
        print("bluebus-py: acquire mode is not supported, using D-Bus I/O", file=sys.stderr)
    print(json.dumps(asyncio.run(run(hci, reads, writes, notifications))))


if __name__ == "__main__":
    main(sys.argv)
