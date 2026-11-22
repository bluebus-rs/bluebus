# bluebus for C and C++

The high-level `bluebus::ble` API as a C library (`include/bluebus.h`) with
a header-only C++17 wrapper (`include/bluebus.hpp`): Bluetooth Low Energy on
Linux through BlueZ over D-Bus. Scan, connect, pair, read / write / subscribe
to GATT characteristics, and run a GATT server with an LE advertisement.
Linux only (BlueZ 5.x, `bluetoothd` running).

```c
#include "bluebus.h"

bluebus_session *session;
bluebus_adapter *adapter;
bluebus_device *device;
bluebus_session_new(&session);
bluebus_session_default_adapter(session, &adapter);
bluebus_adapter_power_on(adapter);

bluebus_scan_filter filter = {0};
filter.name_prefix = "Sensor";
if (bluebus_adapter_find_device(adapter, &filter, 10000, &device) != BLUEBUS_OK) {
    fprintf(stderr, "%s\n", bluebus_last_error());
    return 1;
}
bluebus_device_connect(device, 20000);       /* waits for service discovery */

uint8_t *data; size_t len;
bluebus_device_read(device, "2a19", &data, &len);
printf("battery: %u%%\n", data[0]);
bluebus_bytes_free(data);

bluebus_device_free(device);
bluebus_adapter_free(adapter);
bluebus_session_free(session);
```

```cpp
#include "bluebus.hpp"

bluebus::Session session;
auto adapter = session.default_adapter();
adapter.power_on();
auto device = adapter.find_device(bluebus::ScanFilter().name_prefix("Sensor"), 10000);
device.connect(20000);
std::vector<uint8_t> level = device.read("2a19");
auto sub = device.subscribe("2a37", [](const std::vector<uint8_t> &v) {
    std::printf("heart rate %u\n", v.at(1));       // on a library thread
});
device.write("2a39", {1});                          // throws bluebus::Error on failure
```

## The API in short

| Area | Functions |
|---|---|
| Session | `bluebus_session_new`, `_adapters`, `_adapter` (name, path or MAC), `_default_adapter`, `_set_agent_auto`, `_set_agent_passkey`, `_remove_agent` |
| Adapter | `bluebus_adapter_power_on` / `_off` / `_is_powered`, `_scan` (blocking, returns `bluebus_device_info[]`), `_scan_start` (callback) + `bluebus_scan_stop`, `_find_device`, `_device`, `_devices`, `_remove_device`, `_name` / `_path` / `_address` |
| Device | `bluebus_device_connect`, `_disconnect`, `_is_connected`, `_pair`, `_trust`, `_get_info`, `_rssi`, `_battery_level`, `_services` (flattened `bluebus_gatt_attribute[]`), `_read`, `_write`, `_read_descriptor`, `_write_descriptor` |
| Notifications | `bluebus_device_subscribe` (callback) or `bluebus_device_notifications` + `bluebus_notifications_next` (pull, with timeout), `bluebus_notifications_stop` |
| Peripheral | `bluebus_peripheral_builder_new`, `_add_service`, `_add_characteristic` (`BLUEBUS_FLAG_*`, initial value, write callback), `_add_descriptor`, `_advertise`, `_advertise_manufacturer_data`, `_start`; `bluebus_peripheral_set_value`, `_notify`, `_value`, `_is_notifying`, `_is_running`, `_stop` |
| Misc | `bluebus_last_error`, `bluebus_last_dbus_error_name`, `bluebus_status_str`, `bluebus_version`, `bluebus_normalize_uuid`, `bluebus_bytes_free`, `bluebus_*_info_free` |

The header documents every function (Doxygen comments). The rules:

* **Blocking and thread-safe.** Every call blocks until BlueZ answered; the
  library runs its own runtime internally. Handles may be shared between
  threads. Don't call bluebus from inside a signal handler.
* **Callbacks run on library threads** (one per scan, subscription or
  peripheral), never on the thread that registered them. They may call
  bluebus functions, including stopping or freeing their own handle. Their
  pointer arguments are valid only during the call.
* **Errors.** Fallible functions return `bluebus_status` (`BLUEBUS_OK`,
  `BLUEBUS_ERR_TIMEOUT`, `_NOT_FOUND`, `_INVALID_ARGUMENT`, `_DBUS`,
  `_CLOSED`, `_INTERNAL`). `bluebus_last_error()` gives the message and
  `bluebus_last_dbus_error_name()` the D-Bus name (such as
  `org.bluez.Error.NotReady`) of the last failure on the calling thread.
  NULL handles and malformed arguments return `BLUEBUS_ERR_INVALID_ARGUMENT`;
  no Rust panic crosses the C boundary.
* **Memory.** Every handle has a `_free` (accepting NULL); byte buffers are
  freed with `bluebus_bytes_free`, arrays with their `_free(array, count)`.
* **UUIDs** may be short (`"2a19"`, `"0x2A19"`) or full; results use the
  full lower-case form.
* BlueZ also reports values that this program wrote or read as value
  changes, so a subscription sees them besides the remote notifications.
* A subscription does not end when the device disconnects: watch
  `bluebus_device_is_connected` and subscribe again after reconnecting.

## Prebuilt libraries

Releases tagged `c-v*` carry `bluebus-c-linux-x86_64.tar.gz` and
`bluebus-c-linux-aarch64.tar.gz` (glibc 2.35+), each with

```
include/bluebus.h  include/bluebus.hpp
lib/libbluebus.so  lib/libbluebus.a
lib/cmake/bluebus/bluebusConfig.cmake    (find_package)
lib/pkgconfig/bluebus.pc                 (pkg-config)
```

Unpack it anywhere (it is relocatable) and point your build at it:

```cmake
find_package(bluebus 0.1 REQUIRED)      # -DCMAKE_PREFIX_PATH=/path/to/bluebus-c-linux-x86_64
target_link_libraries(app PRIVATE bluebus::bluebus)   # or bluebus::static
```

```sh
export PKG_CONFIG_PATH=/path/to/bluebus-c-linux-x86_64/lib/pkgconfig
cc app.c $(pkg-config --cflags --libs bluebus)
```

The shared library is named `libbluebus.so` (soname); ship it next to your
program or set an rpath. The static library needs `-lpthread -ldl -lm -lrt`
(`pkg-config --static --libs bluebus`, or `bluebus::static` in CMake).

## Building from source

You need Rust ([rustup.rs](https://rustup.rs)), CMake 3.16+ and a C/C++
compiler. Inside your CMake project:

```cmake
add_subdirectory(path/to/bluebus/capi bluebus)
target_link_libraries(app PRIVATE bluebus::bluebus)
```

CMake runs cargo and links the shared library (`-DBLUEBUS_STATIC=ON` for the
static one). Or build and install it:

```sh
cmake -S capi -B build -DCMAKE_INSTALL_PREFIX=/opt/bluebus
cmake --build build
cmake --install build          # include/, lib/, lib/cmake/bluebus, lib/pkgconfig
```

Without CMake:

```sh
cargo build --release -p bluebus-c   # target/release/libbluebus.{so,a}
cc app.c -Icapi/include -Ltarget/release -lbluebus -Wl,-rpath,$PWD/target/release
```

## Examples

* [`examples/scan.c`](examples/scan.c): callback scan, then the devices
  BlueZ knows.
* [`examples/peripheral.c`](examples/peripheral.c): GATT server with a
  read/write/notify characteristic and an advertisement; answers every
  write with a notification from inside the write callback.
* [`examples/central.cpp`](examples/central.cpp): finds that peripheral,
  connects, reads, writes, receives notifications (callback and pull),
  pairs and disconnects.

Build them with `-DBLUEBUS_EXAMPLES=ON`. Between two adapters (for example
two virtual controllers from BlueZ' `btvirt`, see the root README):

```sh
./bluebus_peripheral hci1 60 &
./bluebus_central hci2 bluebus-c-e2e     # ALL 11 C++ CENTRAL CHECKS PASSED
```

## Tests

```sh
cargo test -p bluebus-c
```

runs the C functions from Rust against the mock BlueZ of the crate's tests
(a private `dbus-daemon`, no Bluetooth hardware), checks that `bluebus.h`
compiles as C99/C11/C17/C++11/C++17 and matches the library (exported
symbols, struct layouts, constants), and compiles and runs
[`tests/test.c`](tests/test.c) (shared and static) and
[`tests/test.cpp`](tests/test.cpp) against the mock. With CMake:

```sh
cmake -S capi -B build -DBLUEBUS_TESTS=ON && cmake --build build && ctest --test-dir build
```

CTest runs each program through the `mock_bluez` cargo example
(`cargo run -p bluebus-c --example mock_bluez -- <program>`), which starts
the mock and points `DBUS_SYSTEM_BUS_ADDRESS` at it.
