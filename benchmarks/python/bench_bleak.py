"""bench_bleak.py <hciN> <reads> <writes> <notifications>: same scenario as bench-bluebus.

Startup includes importing bleak (Python users pay it on every start).
"""
import time

T0 = time.perf_counter()
import asyncio  # noqa: E402
import json  # noqa: E402
import sys  # noqa: E402

from bleak import BleakClient, BleakScanner  # noqa: E402

READ = "b1eb0000-0000-4000-8000-00000000be01"
WRITE = "b1eb0000-0000-4000-8000-00000000be02"
NOTIFY = "b1eb0000-0000-4000-8000-00000000be03"
NAME = "bench-periph"


def ms(s):
    return s * 1e3


def pct(v, p):
    v = sorted(v)
    return ms(v[int((len(v) - 1) * p)])


async def main():
    hci, reads, writes, notes = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
    startup = time.perf_counter() - T0

    t = time.perf_counter()
    device = await BleakScanner.find_device_by_name(NAME, timeout=60, bluez={"adapter": hci})
    discover = time.perf_counter() - t
    assert device is not None

    t = time.perf_counter()
    client = BleakClient(device, bluez={"adapter": hci})
    await client.connect()  # connects and resolves services
    connect = time.perf_counter() - t

    cpu_rw = time.process_time()
    read_t = []
    for _ in range(reads):
        t = time.perf_counter()
        v = await client.read_gatt_char(READ)
        read_t.append(time.perf_counter() - t)
        assert len(v) == 20
    write_t = []
    for _ in range(writes):
        t = time.perf_counter()
        await client.write_gatt_char(WRITE, bytes([1] * 20), response=True)
        write_t.append(time.perf_counter() - t)

    cpu_rw = time.process_time() - cpu_rw
    got = 0
    last = 0.0
    done = asyncio.Event()

    def on_note(_, __):
        nonlocal got, last
        got += 1
        last = time.perf_counter() - t
        if got >= notes:
            done.set()

    acquire = len(sys.argv) > 5 and sys.argv[5] == "acquire"
    await client.start_notify(NOTIFY, on_note, bluez={"use_start_notify": not acquire})
    t = time.perf_counter()
    cpu_n = time.process_time()
    await client.write_gatt_char(NOTIFY, notes.to_bytes(4, "little"), response=True)
    while got < notes:
        before = got
        try:
            await asyncio.wait_for(done.wait(), 3)
        except asyncio.TimeoutError:
            if got == before:
                break
    cpu_n = time.process_time() - cpu_n
    await client.stop_notify(NOTIFY)

    t = time.perf_counter()
    await client.disconnect()
    disconnect = time.perf_counter() - t

    print(json.dumps({
        "lib": "bleak", "startup_ms": round(ms(startup), 2), "discover_ms": round(ms(discover), 1),
        "connect_ms": round(ms(connect), 1),
        "read_p50_ms": round(pct(read_t, 0.5), 3), "read_p95_ms": round(pct(read_t, 0.95), 3),
        "write_p50_ms": round(pct(write_t, 0.5), 3), "write_p95_ms": round(pct(write_t, 0.95), 3),
        "notify_received": got, "notify_ms": round(ms(last), 1), "disconnect_ms": round(ms(disconnect), 1),
        "cpu_rw_ms": round(ms(cpu_rw), 2), "cpu_notify_ms": round(ms(cpu_n), 2),
    }))


asyncio.run(main())
