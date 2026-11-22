# bluebus for Python

Asyncio Bluetooth Low Energy for **Linux**: scan, connect, read, write and
subscribe to GATT characteristics, run a **GATT server with advertising**
(BLE peripheral) and **pair** with a pairing agent. It talks to the BlueZ
daemon over D-Bus through the [bluebus](https://crates.io/crates/bluebus)
Rust crate; every I/O method is an `async def`-style awaitable.

```sh
pip install bluebus
```

One abi3 wheel per architecture (x86_64, aarch64) covers Python 3.9 and
newer. No Python dependencies, no `dbus-python`, no GLib.

## Quick start

```python
import asyncio
import bluebus

async def main():
    session = await bluebus.Session.new()          # or: await bluebus.Session()
    adapter = await session.default_adapter()      # or: await session.adapter("hci1")
    await adapter.power_on()

    for d in await adapter.scan(timeout=5.0, transport="le"):
        print(d.address, d.name, d.rssi, d.manufacturer_data)

    device = await adapter.find_device(name="Sensor", timeout=10.0)
    async with device.connected(timeout=20.0):     # waits for service discovery
        battery: bytes = await device.read("2a19")
        print("battery", battery[0], "%")
        await device.write("2a06", b"\x01", response=True)
        async for value in device.notifications("2a37"):
            print("heart rate", value[1])
            break

asyncio.run(main())
```

Characteristics are addressed by UUID: `"2a19"`, `"0x2A19"` or the full
form `"00002a19-0000-1000-8000-00805f9b34fb"`.

## Scanning

```python
# Collect for a while (devices sorted by address).
devices = await adapter.scan(timeout=5.0, name_prefix="Sensor", service_uuids=["180d"], min_rssi=-80)

# Or react to each advertisement as it arrives.
async for event in adapter.scan_stream(transport="le"):
    print(event.kind, event.device)          # kind: "found" | "updated"
    if event.device.name == "Sensor":
        break                                # discovery stops

# Explicit lifetime:
async with adapter.scan_stream(service_uuids=["180f"]) as events:
    first = await events.__anext__()
```

`DeviceInfo` has read-only attributes: `address`, `address_type`, `name`,
`alias`, `rssi`, `tx_power`, `uuids`, `manufacturer_data` (`{company_id:
bytes}`), `service_data` (`{uuid: bytes}`), `connected`, `paired`,
`trusted`, `services_resolved`.

## GATT client

```python
device = adapter.device("AA:BB:CC:DD:EE:FF")   # no I/O, or: await adapter.find_device(...)
await device.connect(timeout=20.0)
for service in await device.services():
    for c in service.characteristics:
        print(service.uuid, c.uuid, c.flags, [d.uuid for d in c.descriptors])

value = await device.read("2a00")
await device.write("6e400002-b5a3-f393-e0a9-e50e24dcca9e", b"hi", response=False)
await device.read_descriptor("2a19", "2902")

async with device.notifications("2a37") as values:   # subscribed here
    await device.write("2a39", b"\x01")              # trigger something
    async for v in values:                           # in arrival order
        ...
await device.disconnect()
```

A notification stream ends when it is closed (`aclose()`, `async with`
exit, `break`); several streams on one characteristic share one BlueZ
subscription.

## Peripheral (GATT server + advertising)

```python
def on_write(value: bytes):          # may also be an `async def`
    print("written:", value)

peripheral = await (
    adapter.peripheral()
    .service(bluebus.Service("180f").characteristic(
        bluebus.Characteristic("2a19", read=True, notify=True, value=b"\x64")))
    .service(bluebus.Service("b1eb0000-0000-4000-8000-000000000001").characteristic(
        bluebus.Characteristic("b1eb0000-0000-4000-8000-000000000002",
                               write=True, on_write=on_write).descriptor("2901", b"control")))
    .advertise(bluebus.Advertisement(local_name="my-sensor", service_uuids=["180f"],
                                     manufacturer_data={0xFFFF: b"\x01\x02"}))
    .start()
)
await peripheral.notify("2a19", b"\x63")   # value + notification to subscribers
await peripheral.stop()                    # or: async with peripheral: ...
```

`on_write` callbacks run on the asyncio event loop that called `start()`,
in the order the writes arrived. Coroutine callbacks are scheduled as
tasks. A callback that raises is logged on the `"bluebus"` logger; BlueZ
and the Rust runtime never wait for Python code.

## Pairing

```python
await session.set_agent("auto")            # accept "Just Works" / confirmations
await session.set_agent(passkey=123456)    # answer passkey (and PIN) requests
await device.pair()
await device.trust(True)
await session.remove_agent()
```

## Errors

| Exception | Raised for | Also a |
|---|---|---|
| `bluebus.BluebusError` | base class of all errors | `Exception` |
| `bluebus.BluebusTimeout` | scan/connect/power waits | `TimeoutError` |
| `bluebus.NotFound` | unknown adapter, device, characteristic | `LookupError` |
| `bluebus.InvalidArgument` | bad UUID, MAC address, timeout, … | `ValueError` |
| `bluebus.DBusError` | BlueZ / D-Bus errors; `.name` is e.g. `"org.bluez.Error.NotReady"` | |

Awaitables can be cancelled (`asyncio.wait_for`, task cancellation); the
Rust operation is dropped with them.

## Compared to bleak

[bleak](https://github.com/hbldh/bleak) is cross-platform (Windows, macOS,
Linux, Android) and central-only. bluebus is **Linux-only** (BlueZ; tested with 5.72)
and adds what bleak does not have on Linux:

- a **GATT server** (peripheral role) with write callbacks and
  notifications, and **LE advertising** (local name, service UUIDs,
  manufacturer and service data, broadcast beacons);
- a **pairing agent** (`set_agent("auto")`, `set_agent(passkey=…)`);
- scan filters applied on both sides (BlueZ discovery filter + local
  name/address/RSSI matching), `find_device` and `scan_stream` events.

The API shape is close to bleak's (`async with`, UUID strings, `bytes`
values), so a Linux central ports over with few changes.

## Requirements

- Linux with BlueZ (`bluetoothd`) running and permission to use it on the
  system bus (desktop users have it; otherwise the `bluetooth` group).
- `DBUS_SYSTEM_BUS_ADDRESS` is honoured, e.g. to point at a test bus.

## Development

```sh
cd python
uv venv && uv pip install maturin pytest pytest-asyncio
maturin develop --uv
pytest -q tests          # against the mock BlueZ (needs dbus-daemon)
python examples/e2e.py hci1 hci2   # real BlueZ, two (virtual) controllers
```

The tests start `examples/mock_bluez.rs` (a mock `org.bluez` on a private
`dbus-daemon`, shared with the Rust test suite); set `BLUEBUS_MOCK` to a
prebuilt binary to skip `cargo build`.

## License

MIT or Apache-2.0, like the bluebus crate.
