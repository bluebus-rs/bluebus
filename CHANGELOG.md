# Changelog

## Unreleased

Additive only; backward compatible with 0.1.11 and 0.1.10.

### Added
- **`bluebus::ble`: a high-level, easy API** (not re-exported at the crate
  root, where `Adapter`/`Device` are the zbus proxies). Cloneable,
  `Send + Sync + 'static` handles with plain-data results and a single
  `ble::Error` (`Bus`, `Timeout`, `NotFound`, `InvalidArgument`):
  - `Session` (`new`, `with_connection`, `adapters`, `adapter`,
    `default_adapter`, `set_agent`, `remove_agent`);
  - `Adapter` (`power_on`/`power_off`, `scan`, `scan_stream` with a guard that
    stops discovery on drop, `find_device`, `device`, `devices`,
    `remove_device`, `peripheral`) and the `ScanFilter` builder (transport,
    exact/prefix name, address, service UUIDs, minimum RSSI, duplicate data);
  - `Device` (`connect` waiting for `ServicesResolved`, `disconnect`, `pair`,
    `trust`, `services`, `read`/`write` with `WriteType`, `notifications`
    stream that stops on drop and is shared between subscribers,
    `read_descriptor`/`write_descriptor`, `battery_level`, `rssi`, `info`);
    the GATT database is cached per connection and refreshed when stale;
  - `PeripheralBuilder` → `Peripheral`: GATT services, characteristics
    (flags, initial value, write callback, read-only descriptors) and an
    advertisement on unique object paths, with `set_value`, `notify`,
    `value`, `is_notifying` and `stop` (also best effort on drop);
  - `AutoAcceptAgent`, `PasskeyAgent`; `normalize_uuid` (16/32/128-bit forms).
- `examples/ble_e2e.rs`: the virtual-controller end-to-end test written with
  `bluebus::ble` only.
- `characteristic_notifications_acquired`: notifications through BlueZ'
  `AcquireNotify` socket instead of D-Bus signals. 5 000 notifications
  arrive in ~85 ms instead of ~420 ms and cost ~7 ms of CPU instead of ~70 ms
  (virtual controllers, see `benchmarks/`).

- **Python bindings** (`python/`, PyPI package `bluebus`, crate
  `bluebus-python` in a new cargo workspace; the `bluebus` package itself is
  unchanged): PyO3 + `pyo3-async-runtimes` (tokio), abi3 wheels for Python
  3.9+, Linux only. Every I/O method returns an asyncio awaitable:
  `Session` (`await Session.new()` / `await Session()`, `adapters`,
  `adapter`, `set_agent("auto" | passkey=…)`), `Adapter` (`scan`,
  `scan_stream` async iterator, `find_device`, …), `Device` (`connected()`
  async context manager, `read`/`write`, `notifications` async iterator,
  `services`, `pair`, …) and the peripheral builders (`Service`,
  `Characteristic` with `on_write` callbacks run on the event loop,
  `Advertisement`, `Peripheral`). Read-only `DeviceInfo`/`ServiceInfo`/…
  data classes, type stubs (`py.typed`) and the exceptions `BluebusError`,
  `BluebusTimeout` (a `TimeoutError`), `NotFound`, `InvalidArgument` and
  `DBusError` (`.name`). Tested with pytest against the mock BlueZ;
  `python/examples/e2e.py` mirrors `ble_e2e.rs`; CI in
  `.github/workflows/python.yml` (PyPI trusted publishing on `py-v*` tags).
- `examples/mock_bluez.rs`: the mock `org.bluez` of the test suite as a
  standalone process (prints the private bus address, takes commands on
  stdin) for tests in other languages.

- **C and C++ API** (`capi/`, crate `bluebus-c` 0.1.0, library `libbluebus`
  shared and static; the repository is now a cargo workspace, the `bluebus`
  package is unchanged): blocking, thread-safe C functions over
  `bluebus::ble` in `capi/include/bluebus.h` (opaque handles for session,
  adapter, device, scan, notifications, peripheral builder and peripheral;
  `bluebus_status` codes with thread-local `bluebus_last_error` /
  `bluebus_last_dbus_error_name`; scanning blocking or with callbacks,
  connect, pair, trust, GATT read/write/descriptors, flattened service
  list, notifications as callbacks or pulled with a timeout, a peripheral
  builder with write callbacks, auto-accept and passkey agents), plus the
  header-only C++17 wrapper `bluebus.hpp`. Panics never cross the C
  boundary. CMake project exporting `bluebus::bluebus` (also as an
  installed, relocatable `find_package` config), pkg-config `bluebus.pc`,
  examples `scan.c`, `peripheral.c` and `central.cpp`, tests against the
  mock BlueZ from Rust, C and C++, a `c-api` CI job and a `c-v*` release
  workflow with prebuilt Linux x86_64 and aarch64 archives.

### Changed
- Minimum supported Rust version is now declared (`rust-version = "1.89"`)
  and checked in CI; the current dependency versions need 1.89.
- zbus runs on the tokio runtime (`features = ["tokio"]`) instead of its own
  async-io executor thread: about 35 % less CPU for GATT reads and writes.
  bluebus already required tokio.
- `characteristic_notifications` reads the notified value straight from the
  D-Bus message (about 15 % less CPU per notification), with the general
  decoder as fallback.
- `gatt_characteristic_proxy` caches properties lazily: proxies used only to
  read, write or receive notifications no longer re-parse every
  notification into the property cache.

## 0.1.11

Backward compatible with 0.1.10 (verified with `cargo semver-checks` and
`tests/compat_0_1_10.rs`). Several features that could not work before now
do; their D-Bus side changed accordingly.

### Fixed
- **GATT server could not be registered.** `GattService`/`GattCharacteristic`
  exported their properties as D-Bus *methods*, used `s` instead of `o` for
  object paths, and there was no `RegisterApplication`/`ObjectManager`.
  Properties are now real properties with BlueZ' names and signatures, and
  `register_gatt_application[_at]` registers the application.
- **`Agent` could not be registered**: it did not implement `org.bluez.Agent1`.
  It now implements the new `AgentHandler` trait; `register_agent` exports
  and registers it. PINs/passkeys are no longer printed to stdout.
- **`unregister_advertisement` always failed**: wrong method name
  (`UnRegisterAdvertisement`) and a string instead of an object path; the
  exported object was also left behind, so registering again failed.
- **Advertisement property names**: `ServiceUuids`/`SolicitUuids` were not
  recognised by BlueZ, so service UUIDs were never advertised. Unset optional
  properties were exported as empty values (an empty `SecondaryChannel` is
  rejected by BlueZ); they are now omitted. Advertisements were only
  configurable through `Default`; added `new` + `with_*` builders.
- **GATT client property names**: `uuid()` (`Uuid`), `mtu()` (`Mtu`) always
  failed; they now read `UUID`/`MTU`. Added `service_path`, `device_path`,
  `include_paths` with the correct object-path types.
- **Monitor dead-locked** on the first property change (a read guard was held
  while taking the write lock), spun at 100 % CPU when the bus closed, reported
  a device as removed when only e.g. `Battery1` was removed, ignored devices
  that existed before it started, tracked only 3 properties, and leaked one
  task per device. Rewritten with a single subscription.
- `print_avaiable_devices` printed every object; it now prints devices only.
- Mutex poisoning in the path globals no longer panics.
- Race in zbus' object server start-up that could make the first agent /
  GATT / advertisement registration hang forever.
- README and crate-level examples did not compile.

### Added
- Adapter: discovery filter (+ typed `DiscoveryFilter`), `GetDiscoveryFilters`,
  `ConnectDevice`, timeouts, pairable, UUIDs, roles, …; `adapter_proxy`,
  `list_adapters`, `set_adapter_powered`.
- Device: `ConnectProfile`/`DisconnectProfile`, alias, RSSI, TX power,
  manufacturer/service/advertising data, appearance, icon, bonded, wake
  allowed, …; `Battery1`; `device_proxy`, `device_path`,
  `address_from_device_path`, `advertising_bytes`.
- GATT: descriptors (client and server), `AcquireWrite`/`AcquireNotify`/
  `Confirm`, `discover_gatt_database`, `characteristic_notifications` (every
  notification, in order), offset-aware read/write, write callbacks,
  `notify_characteristic_value`, `GattManager1Proxy`.
- `LEAdvertisingManager1Proxy`, `register_advertisement_at`.
- `list_devices_with_connection`, `list_adapter_devices[_with_paths]`,
  `DeviceInfo::from_properties`, `Monitor::run`, `Monitor::devices`.
- Mock-BlueZ integration tests, hardware smoke test, virtual end-to-end test.

### Deprecated
- `print_avaiable_devices` → `print_available_devices`.
