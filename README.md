# bluebus: async Bluetooth and BLE for Rust on Linux (BlueZ, D-Bus)

[![crates.io](https://img.shields.io/crates/v/bluebus.svg)](https://crates.io/crates/bluebus)
[![docs.rs](https://docs.rs/bluebus/badge.svg)](https://docs.rs/bluebus)
[![downloads](https://img.shields.io/crates/d/bluebus)](https://crates.io/crates/bluebus)
[![CI](https://github.com/bluebus-rs/bluebus/actions/workflows/ci.yml/badge.svg)](https://github.com/bluebus-rs/bluebus/actions/workflows/ci.yml)
[![CodeQL](https://github.com/bluebus-rs/bluebus/actions/workflows/codeql.yml/badge.svg)](https://github.com/bluebus-rs/bluebus/actions/workflows/codeql.yml)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/bluebus-rs/bluebus/badge)](https://scorecard.dev/viewer/?uri=github.com/bluebus-rs/bluebus)
[![MSRV](https://img.shields.io/badge/MSRV-1.89-blue)](Cargo.toml)
[![license](https://img.shields.io/crates/l/bluebus)](#license)
[![GitHub stars](https://img.shields.io/github/stars/bluebus-rs/bluebus?style=social)](https://github.com/bluebus-rs/bluebus/stargazers)

**bluebus** is an async Rust library for Bluetooth Classic and Bluetooth Low
Energy (BLE) on Linux. It talks to the **BlueZ** Bluetooth daemon over
**D-Bus** using [zbus](https://crates.io/crates/zbus), so it is pure Rust:
no C library, no `libdbus`, no root access needed.

Use it to **scan** for devices, **connect**, **pair**, read and write **GATT
characteristics**, receive **notifications**, run a **GATT server**
(BLE peripheral), broadcast **LE advertisements** (beacons) and follow device
events, all from `async`/`await` code with `tokio`.

```toml
[dependencies]
bluebus = "0.1.11"
tokio = { version = "1", features = ["full"] }
```

## Easy API

`bluebus::ble` wraps everything below in a few cloneable handles
(`Session`, `Adapter`, `Device`, `Peripheral`), with characteristics
addressed by UUID (`"2a19"` or full form) and one error type:

```rust,no_run
use std::time::Duration;
use bluebus::ble::{Advertisement, Characteristic, ScanFilter, Service, Session, WriteType};
use futures::StreamExt;

# async fn run() -> bluebus::ble::Result<()> {
let adapter = Session::new().await?.default_adapter().await?;
adapter.power_on().await?;

// Central: scan, connect, read, write, notifications.
for d in adapter.scan(ScanFilter::new(), Duration::from_secs(5)).await? {
    println!("{} {:?} rssi={:?}", d.address, d.name, d.rssi);
}
let device = adapter
    .find_device(ScanFilter::new().name_prefix("Sensor"), Duration::from_secs(10))
    .await?;
device.connect(Duration::from_secs(20)).await?; // waits for service discovery
println!("battery: {:?}", device.read("2a19").await?);
device.write("2a06", vec![1], WriteType::WithResponse).await?;
let mut values = device.notifications("2a37").await?; // stops on drop
while let Some(v) = values.next().await {
    println!("{v:02x?}");
}

// Peripheral: GATT server + advertisement.
let peripheral = adapter
    .peripheral()
    .service(Service::new("180f").characteristic(
        Characteristic::new("2a19").read().notify().value(vec![100]),
    ))
    .advertise(Advertisement::new().local_name("my-sensor").service_uuid("180f"))
    .start()
    .await?;
peripheral.notify("2a19", vec![99]).await?;
peripheral.stop().await?;
# Ok(()) }
```

Pairing: `session.set_agent(AgentCapability::NoInputNoOutput, AutoAcceptAgent)`
then `device.pair()`. The rest of this page shows the low-level API, which
stays available (`Session::connection()` gives its D-Bus connection).

## Python

The same API is available from Python (asyncio, Linux only) as the
[`bluebus`](https://pypi.org/project/bluebus/) package, built with PyO3 from
[`python/`](python/):

```sh
pip install bluebus
```

```python
import asyncio, bluebus

async def main():
    adapter = await (await bluebus.Session.new()).default_adapter()
    await adapter.power_on()
    device = await adapter.find_device(name="Sensor", timeout=10.0)
    async with device.connected(timeout=20.0):
        print(await device.read("2a19"))
        async for value in device.notifications("2a37"):
            print(value.hex())

asyncio.run(main())
```

Peripherals (`adapter.peripheral()…start()`, `on_write` callbacks on the
event loop), advertising and pairing agents (`session.set_agent("auto")`)
are included; see [python/README.md](python/README.md).

## Contents

- [Easy API](#easy-api)
- [Python](#python)
- [Features](#features)
- [Requirements](#requirements)
- [Quick start: scan for BLE devices](#quick-start-scan-for-ble-devices)
- [Examples](#examples)
  - [Connect and read/write a GATT characteristic](#connect-and-readwrite-a-gatt-characteristic)
  - [Subscribe to GATT notifications](#subscribe-to-gatt-notifications)
  - [Pair with a device (pairing agent)](#pair-with-a-device-pairing-agent)
  - [Create a BLE peripheral (GATT server)](#create-a-ble-peripheral-gatt-server)
  - [Broadcast an LE advertisement](#broadcast-an-le-advertisement)
  - [Watch devices appear, change and disappear](#watch-devices-appear-change-and-disappear)
- [API overview](#api-overview)
- [Troubleshooting](#troubleshooting)
- [FAQ](#faq)
- [Testing](#testing)
- [Compatibility and versioning](#compatibility-and-versioning)
- [Contributing](#contributing)
- [License](#license)

## Features

| Area | What you can do |
|---|---|
| **Adapter** (`org.bluez.Adapter1`) | Power on/off, discovery with typed filters (LE / BR/EDR, RSSI, UUIDs), alias, discoverable, pairable, multiple adapters (`hci0`, `hci1`, …) |
| **Device** (`org.bluez.Device1`) | Connect, disconnect, connect a single profile, pair, cancel pairing, trust, block, alias, RSSI, TX power, manufacturer data, service data, battery level |
| **Pairing agent** (`org.bluez.Agent1`) | Answer PIN code, passkey, confirmation and authorization requests with your own logic |
| **GATT client** | Discover services, characteristics and descriptors; read and write (with offset and write type); ordered notification stream |
| **GATT server / BLE peripheral** | Export services, characteristics and descriptors; write callbacks; send notifications and indications |
| **LE advertising** | Local name, service UUIDs, manufacturer data (iBeacon-style payloads), service data, intervals, TX power, timeout |
| **Events** | Device added, removed and property changed, including devices that were known before monitoring started |

Everything BlueZ exposes is also reachable through the generated
[zbus](https://crates.io/crates/zbus) proxies (`AdapterProxy`, `DeviceProxy`,
`GattCharacteristic1Proxy`, …).

## Requirements

- **Linux** with **BlueZ** (`bluetoothd`) running. Tested with BlueZ 5.72 on
  Ubuntu 24.04, with a real Intel controller and with virtual controllers.
- A Bluetooth adapter. `rfkill list` must not show it as blocked.
- Permission to use BlueZ on the system bus. Desktop users have it by
  default; on servers or headless systems, add the user to the `bluetooth`
  group or adjust the D-Bus policy.
- A `tokio` runtime.
- **Rust 1.89** or newer (checked in CI).

## Quick start: scan for BLE devices

```rust,no_run
use std::time::Duration;
use bluebus::{DiscoveryFilter, Transport};

#[tokio::main]
async fn main() -> zbus::Result<()> {
    let conn = bluebus::get_system_connection().await?;
    let adapter = bluebus::adapter_proxy(&conn).await?; // default: /org/bluez/hci0

    // Turn the radio on and wait until BlueZ reports it is on.
    bluebus::set_adapter_powered(&adapter, true, Duration::from_secs(10)).await?;

    // Only Low Energy devices, scan for 5 seconds.
    adapter.set_discovery_filter(DiscoveryFilter::new().transport(Transport::Le).to_dict()).await?;
    adapter.start_discovery().await?;
    tokio::time::sleep(Duration::from_secs(5)).await;
    adapter.stop_discovery().await?;

    for device in bluebus::list_devices_with_connection(&conn).await? {
        println!("{}  {:<24} rssi={:?}", device.address, device.alias, device.rssi);
    }
    Ok(())
}
```

## Examples

Each snippet is compiled as part of `cargo test`. The `examples/` directory
contains complete programs.

### Connect and read/write a GATT characteristic

```rust,no_run
use std::time::Duration;

# async fn run() -> zbus::Result<()> {
let conn = bluebus::get_system_connection().await?;
let device = bluebus::device_proxy(&conn, "AA:BB:CC:DD:EE:FF").await?;
device.connect().await?;

// Wait until BlueZ has resolved the remote GATT database.
while !device.services_resolved().await? {
    tokio::time::sleep(Duration::from_millis(100)).await;
}

let db = bluebus::discover_gatt_database(&conn, device.inner().path().as_str()).await?;
let battery_level = db
    .characteristic_by_uuid("00002a19-0000-1000-8000-00805f9b34fb")
    .expect("no Battery Level characteristic");

let chr = bluebus::gatt_characteristic_proxy(&conn, battery_level.path.clone()).await?;
let value = chr.read_value(Default::default()).await?;
println!("battery: {}%", value[0]);

// Writes take the same options BlueZ accepts ("offset", "type", …).
// chr.write_value(vec![0x01], Default::default()).await?;
# Ok(()) }
```

### Subscribe to GATT notifications

`characteristic_notifications` delivers every notification in order, even
when several arrive back to back.

```rust,no_run
use futures::StreamExt;

# async fn run(chr: bluebus::GattCharacteristic1Proxy<'static>) -> zbus::Result<()> {
let mut notifications = bluebus::characteristic_notifications(&chr).await?;
while let Some(value) = notifications.next().await {
    println!("notification: {value:02x?}");
}
chr.stop_notify().await?;
# Ok(()) }
```

### Pair with a device (pairing agent)

Implement `AgentHandler` for the requests your device needs. Every method has
a sensible default: PIN and passkey requests are rejected, confirmations are
accepted. Handlers run on your tokio runtime, so they can await anything.

```rust,no_run
use bluebus::{AgentCapability, AgentHandler, AgentResult};
use zbus::zvariant::OwnedObjectPath;

struct MyAgent;

impl AgentHandler for MyAgent {
    async fn request_passkey(&self, device: OwnedObjectPath) -> AgentResult<u32> {
        println!("passkey requested by {device}");
        Ok(123456)
    }
}

# async fn run() -> zbus::Result<()> {
let conn = bluebus::get_system_connection().await?;
bluebus::register_agent(&conn, "/my/agent", AgentCapability::KeyboardDisplay, MyAgent, true).await?;

let device = bluebus::device_proxy(&conn, "AA:BB:CC:DD:EE:FF").await?;
device.pair().await?;
device.set_trusted(true).await?;

bluebus::unregister_agent(&conn, "/my/agent").await?;
# Ok(()) }
```

For a fixed PIN code and passkey, use the built-in
`bluebus::Agent::new(pin, passkey)` as the handler.

### Create a BLE peripheral (GATT server)

Export the objects below the application path first, then register the
application with BlueZ.

```rust,no_run
use bluebus::{GattCharacteristic, GattService};

# async fn run() -> zbus::Result<()> {
let conn = bluebus::get_system_connection().await?;
let app = bluebus::get_gatt_application_path();
let service = format!("{app}/service0");
let characteristic = format!("{service}/char0");

bluebus::register_service(&conn, &service, GattService {
    uuid: "12345678-1234-5678-1234-56789abcdef0".into(),
    primary: true,
    device: None,
    includes: vec![],
    handle: None,
}).await?;

bluebus::register_characteristic(
    &conn,
    &characteristic,
    GattCharacteristic::new("12345678-1234-5678-1234-56789abcdef1", service.clone(), &["read", "write", "notify"])
        .with_value(*b"hello"),
).await?;

// Called whenever a central writes the characteristic.
bluebus::set_write_handler(&characteristic, |_path, value| println!("written: {value:?}"));

bluebus::register_gatt_application(&conn).await?;

// Push a new value to subscribed centrals.
bluebus::notify_characteristic_value(&conn, &characteristic, b"world".to_vec()).await?;
# Ok(()) }
```

To be found by phones and other centrals, also broadcast an advertisement
(next section).

### Broadcast an LE advertisement

Only the properties you set are sent to BlueZ.

```rust,no_run
use bluebus::{AdvertisementType, LEAdvertisementProperties};

# async fn run() -> zbus::Result<()> {
let conn = bluebus::get_system_connection().await?;
let advertisement = LEAdvertisementProperties::new(AdvertisementType::Peripheral)
    .with_local_name("my-sensor")
    .with_service_uuids(["12345678-1234-5678-1234-56789abcdef0"])
    .with_manufacturer_data(0xFFFF, vec![0x01, 0x02])
    .with_discoverable(true);

bluebus::register_advertisement(&conn, advertisement).await?;
tokio::time::sleep(std::time::Duration::from_secs(30)).await;
bluebus::unregister_advertisement(&conn).await?;
# Ok(()) }
```

Use `AdvertisementType::Broadcast` for a non-connectable beacon.

### Watch devices appear, change and disappear

```rust,no_run
use std::sync::Arc;
use tokio::sync::mpsc;

# async fn run() -> zbus::Result<()> {
let conn = Arc::new(bluebus::get_system_connection().await?);
let manager = Arc::new(bluebus::ObjectManagerProxy::new(&conn).await?);
let (added_tx, mut added) = mpsc::channel(32);
let (removed_tx, mut removed) = mpsc::channel(32);
let (changed_tx, mut changed) = mpsc::channel(32);

let monitor = Arc::new(bluebus::Monitor::new(conn, manager, added_tx, removed_tx, changed_tx).await);
tokio::spawn({
    let monitor = monitor.clone();
    async move { monitor.run().await }
});

loop {
    tokio::select! {
        Some(d) = added.recv() => println!("added   {} {}", d.address, d.alias),
        Some(d) = changed.recv() => println!("changed {} connected={} rssi={:?}", d.address, d.connected, d.rssi),
        Some(d) = removed.recv() => println!("removed {}", d.address),
        else => break,
    }
}
# Ok(()) }
```

## API overview

| Task | Function or type |
|---|---|
| Connect to the system bus | `get_system_connection()` |
| List adapters / pick one | `list_adapters()`, `adapter_proxy()`, `set_adapter_path("/org/bluez/hci1")` |
| Power the adapter | `set_adapter_powered()`, `AdapterProxy::set_powered()` |
| Scan | `AdapterProxy::start_discovery()`, `DiscoveryFilter`, `Transport` |
| List known devices | `list_devices_with_connection()`, `list_adapter_devices_with_paths()`, `DeviceInfo` |
| Control a device | `device_proxy()`, `DeviceProxy` (`connect`, `pair`, `set_trusted`, `connect_profile`, …) |
| Battery level | `BatteryProxy` |
| Device path ↔ MAC address | `device_path()`, `address_from_device_path()` |
| Pairing | `AgentHandler`, `register_agent()`, `unregister_agent()`, `Agent` |
| GATT client | `discover_gatt_database()`, `gatt_characteristic_proxy()`, `gatt_descriptor_proxy()`, `characteristic_notifications()` |
| GATT server | `GattService`, `GattCharacteristic`, `GattDescriptor`, `register_gatt_application()`, `set_write_handler()`, `notify_characteristic_value()` |
| Advertising | `LEAdvertisementProperties`, `register_advertisement()`, `register_advertisement_at()`, `LEAdvertisingManager1Proxy` |
| Events | `Monitor` |
| **Easy API** | `ble::Session`, `ble::Adapter`, `ble::Device`, `ble::Peripheral`, `ble::ScanFilter` |

Functions without an adapter argument use the adapter set with
`set_adapter_path` (default `/org/bluez/hci0`). Each of them has an `_at`
variant or an explicit-path equivalent for multi-adapter programs.

> **Property cache:** proxy getters such as `adapter.powered()` read zbus'
> property cache, which BlueZ updates shortly *after* a setter returns. To read
> the new value right away, wait for it (`set_adapter_powered` does this) or
> build the proxy with `.cache_properties(zbus::proxy::CacheProperties::No)`.

Full documentation: **[docs.rs/bluebus](https://docs.rs/bluebus)**.

## Troubleshooting

| Error or symptom | Cause and fix |
|---|---|
| `org.bluez.Error.NotReady` | The adapter is off. Call `set_adapter_powered(&adapter, true, …)`. |
| `org.bluez.Error.Failed` / `Blocked` when powering on | The radio is rfkill-blocked. Run `rfkill unblock bluetooth`. |
| `org.freedesktop.DBus.Error.ServiceUnknown` (org.bluez) | `bluetoothd` is not running. Run `sudo systemctl start bluetooth`. |
| `org.freedesktop.DBus.Error.AccessDenied` | Your user may not talk to BlueZ. Add it to the `bluetooth` group or check the D-Bus policy. |
| `UnknownObject` for `/org/bluez/hci0` | No such adapter. Check `list_adapters()`. |
| No devices found | Scan longer, check `rfkill`, and remove filters (`DiscoveryFilter::new()` resets them). |
| `ServicesResolved` stays `false` | The device is not connected or has no GATT database. Make sure `connect()` succeeded. |
| `PowerState` returns an error | Not provided by older BlueZ versions (missing in 5.72). Use `powered()`. |
| `MinInterval`, `MaxInterval`, `TxPower` advertisement fields ignored | BlueZ treats them as experimental. Start `bluetoothd --experimental`. |

## FAQ

**How do I scan for BLE devices in Rust on Linux?**
Power on the adapter, set a `DiscoveryFilter` with `Transport::Le`, call
`start_discovery()`, then list devices with `list_devices_with_connection()`
or receive them live with `Monitor`. See the [quick start](#quick-start-scan-for-ble-devices).

**Can I build a BLE peripheral or GATT server in Rust?**
Yes. Export `GattService` and `GattCharacteristic` objects, call
`register_gatt_application()` and advertise with `register_advertisement()`.

**Does it work on a Raspberry Pi or other embedded Linux?**
It runs wherever BlueZ runs, because it only talks to BlueZ over D-Bus.
It has been tested on x86-64 Ubuntu.

**Does it support Windows or macOS?**
No. bluebus is a BlueZ client, so it is Linux only. For cross-platform BLE
central support, see [btleplug](https://crates.io/crates/btleplug).

**How is it different from `bluer`?**
[bluer](https://crates.io/crates/bluer) is the official BlueZ binding and is
built on `libdbus` (the `dbus` crate). bluebus is built on `zbus`, a pure Rust
D-Bus implementation, and exposes BlueZ' interfaces directly as zbus proxies.

**Do I need root?**
No. Normal desktop users can use BlueZ. Only creating virtual controllers
for testing (`/dev/vhci`) needs elevated rights.

## Testing

```sh
cargo test
```

`cargo test` runs:
- unit tests;
- every README and API documentation example (compiled);
- a check that code written for 0.1.10 still compiles;
- integration tests against a mock BlueZ service on a private `dbus-daemon`.

No Bluetooth hardware is needed; the only requirement is `dbus-daemon`.

Against real hardware and the real daemon:

```sh
# Non-destructive checks on an adapter (no pairing, no connections).
cargo run --example hardware_smoke            # hci0
cargo run --example hardware_smoke -- hci1

# Full peripheral <-> central test between two virtual controllers created
# with BlueZ' btvirt: advertise, discover, connect, GATT read/write/notify, pair.
docker build -t bluebus-btvirt tools/btvirt
docker run -d --rm --name bluebus-vhci --privileged bluebus-btvirt -l2
cargo run --example virtual_e2e -- hci1 hci2
cargo run --example ble_e2e -- hci1 hci2      # the same with bluebus::ble
docker stop bluebus-vhci
```

## Compatibility and versioning

bluebus follows [semantic versioning](https://semver.org/). 0.1.11 is
backward compatible with 0.1.10: this is checked with `cargo semver-checks`
and a compile test. See [CHANGELOG.md](CHANGELOG.md) for the bugs fixed
in 0.1.11.

## Contributing

Issues and pull requests are welcome on
[GitHub](https://github.com/bluebus-rs/bluebus); see
[CONTRIBUTING.md](CONTRIBUTING.md). Security issues: [SECURITY.md](SECURITY.md).

## License

Licensed under either of the MIT License or the Apache License, Version 2.0,
at your option. See [LICENSE](LICENSE).

## Acknowledgments

Special thanks to [REM Vision Lab](https://remvisionlab.com/) and
[Sezai Acer](https://github.com/sezaiacers) for their support in improving
this library.
