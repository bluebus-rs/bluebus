// C++ test of bluebus.hpp against the mock BlueZ of the Rust tests (run by
// `cargo test -p bluebus-c --test c_programs` or CTest). Exit status 0 means
// every check passed.
#include <atomic>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <mutex>
#include <set>
#include <thread>

#include "bluebus.hpp"

static int checks = 0;

#define CHECK(cond)                                                                       \
    do {                                                                                  \
        ++checks;                                                                         \
        if (!(cond)) {                                                                    \
            std::fprintf(stderr, "%s:%d: CHECK(%s) failed\n", __FILE__, __LINE__, #cond); \
            std::exit(1);                                                                 \
        }                                                                                 \
    } while (0)

template <typename F> static bool eventually(F &&f) {
    for (int i = 0; i < 500; ++i) {
        if (f()) return true;
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
    return false;
}

template <typename F> static bluebus_status error_of(F &&f) {
    try {
        f();
    } catch (const bluebus::Error &e) {
        return e.status();
    }
    return BLUEBUS_OK;
}

int main() try {
    CHECK(bluebus::version() == BLUEBUS_VERSION_STRING);
    CHECK(bluebus::normalize_uuid("0x2A19") == "00002a19-0000-1000-8000-00805f9b34fb");
    CHECK(error_of([] { bluebus::normalize_uuid("nope"); }) == BLUEBUS_ERR_INVALID_ARGUMENT);

    bluebus::Session session;
    auto adapters = session.adapters();
    CHECK(adapters.size() == 2 && adapters[1].name == "hci1");
    CHECK(error_of([&] { session.adapter("hci9"); }) == BLUEBUS_ERR_NOT_FOUND);

    bluebus::Adapter adapter = session.default_adapter();
    CHECK(adapter.name() == "hci0");
    try {
        adapter.scan(bluebus::ScanFilter(), 10);
        CHECK(false);
    } catch (const bluebus::Error &e) {
        CHECK(e.status() == BLUEBUS_ERR_DBUS);
        CHECK(e.dbus_name() == "org.bluez.Error.NotReady");
    }
    adapter.power_on();
    CHECK(adapter.is_powered());

    auto found = adapter.scan(bluebus::ScanFilter().min_rssi(-60), 100);
    CHECK(found.size() == 1);
    CHECK(found[0].name.value_or("") == "Headset");
    CHECK(found[0].rssi.value_or(0) == -48);

    {
        std::mutex m;
        std::set<std::string> seen;
        bluebus::Scan scan =
            adapter.scan_start(bluebus::ScanFilter(), [&](const bluebus::DeviceInfo &d, bluebus::ScanEvent) {
                std::lock_guard<std::mutex> lock(m);
                seen.insert(d.address);
            });
        CHECK(eventually([&] {
            std::lock_guard<std::mutex> lock(m);
            return seen.size() == 2;
        }));
        scan.stop();
    }  // destroyed: stopped again (no-op), callback released

    bluebus::Device device = adapter.find_device(bluebus::ScanFilter().name("Headset"), 5000);
    CHECK(device.address() == "11:22:33:44:55:66");
    device.connect(5000);
    CHECK(device.is_connected());
    CHECK(device.read("2a19") == std::vector<uint8_t>{87});
    CHECK(device.battery_level() == 87);
    CHECK(device.rssi().value_or(0) == -48);
    CHECK(error_of([&] { device.read("2a37"); }) == BLUEBUS_ERR_NOT_FOUND);
    auto services = device.services();
    CHECK(services.size() == 3);
    CHECK(services[1].kind == BLUEBUS_ATTR_CHARACTERISTIC);
    CHECK(services[1].flags & BLUEBUS_FLAG_READ);
    CHECK(services[2].characteristic_uuid.has_value());
    CHECK(device.read_descriptor("2a19", "2902") == (std::vector<uint8_t>{0, 0}));

    std::atomic<int> notified{0};
    std::atomic<int> last{0};
    {
        bluebus::Notifications sub = device.subscribe("2a19", [&](const std::vector<uint8_t> &v) {
            last = v.empty() ? -1 : v[0];
            ++notified;
            throw std::runtime_error("ignored by the wrapper");
        });
        bluebus::Notifications pull = device.notifications("2a19");
        CHECK(!pull.next(20).has_value());
        device.write("2a19", {7}, true);
        CHECK(eventually([&] { return notified.load() == 1; }));
        CHECK(last.load() == 7);
        auto v = pull.next(2000);
        CHECK(v.has_value() && *v == std::vector<uint8_t>{7});
        // Moving keeps the subscription alive.
        bluebus::Notifications moved = std::move(sub);
        device.write("2a19", {8}, false);
        CHECK(eventually([&] { return notified.load() == 2; }));
        pull.stop();
        CHECK(error_of([&] { pull.next(10); }) == BLUEBUS_ERR_CLOSED);
    }

    session.set_agent_auto();
    device.pair();
    device.trust();
    auto info = device.info();
    CHECK(info.paired && info.trusted && info.connected && info.services_resolved);
    session.remove_agent();
    device.disconnect();
    CHECK(!device.is_connected());

    {
        auto builder = adapter.peripheral();
        builder.service("180f")
            .characteristic("180f", "2a19", BLUEBUS_FLAG_READ | BLUEBUS_FLAG_NOTIFY, {100})
            .characteristic("180f", "12345678-1234-5678-1234-56789abcdef1", BLUEBUS_FLAG_WRITE, {},
                            [](const std::vector<uint8_t> &) {})
            .descriptor("2a19", "2901", {'l', 'v', 'l'})
            .advertise("bluebus-cpp", {"180f"})
            .manufacturer_data(0xffff, {1, 2});
        CHECK(error_of([&] { builder.characteristic("1234", "2a00", BLUEBUS_FLAG_READ); }) ==
              BLUEBUS_ERR_NOT_FOUND);
        bluebus::Peripheral peripheral = builder.start();
        CHECK(peripheral.is_running());
        peripheral.set_value("2a19", {50});
        CHECK(peripheral.value("2a19") == std::vector<uint8_t>{50});
        peripheral.notify("2a19", {49});
        CHECK(!peripheral.is_notifying("2a19"));
        peripheral.stop();
        CHECK(!peripheral.is_running());
        CHECK(error_of([&] { peripheral.set_value("2a19", {1}); }) == BLUEBUS_ERR_INVALID_ARGUMENT);
    }

    std::printf("all %d C++ checks passed\n", checks);
    return 0;
} catch (const bluebus::Error &e) {
    std::fprintf(stderr, "unexpected bluebus::Error %d: %s\n", e.status(), e.what());
    return 1;
}
