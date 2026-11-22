# Changelog

## Unreleased

### Changed
- Minimum supported Rust version is now declared (`rust-version = "1.89"`)
  and checked in CI; the current dependency versions need 1.89.

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
