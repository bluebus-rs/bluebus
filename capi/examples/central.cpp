// A BLE central in C++ (bluebus.hpp): finds the peripheral of
// examples/peripheral.c, connects, reads, writes, receives notifications,
// pairs, then tells the peripheral to quit.
//
//   c++ -std=c++17 central.cpp -Iinclude -Ltarget/release -lbluebus -o central
//   ./central [adapter] [peripheral name]      e.g. ./central hci2 bluebus-c-e2e
#include <chrono>
#include <condition_variable>
#include <cstdio>
#include <mutex>
#include <string>

#include "bluebus.hpp"

static const char *CHAR_UUID = "b1eb05e0-0000-4000-8000-00000000c0e1";

static int checks = 0;
#define EXPECT(cond, what)                                                     \
    do {                                                                       \
        if (!(cond)) {                                                         \
            std::fprintf(stderr, "FAILED: %s\n", what);                        \
            return 1;                                                          \
        }                                                                      \
        std::printf("  ok  %s\n", what);                                       \
        ++checks;                                                              \
    } while (0)

static std::string text(const std::vector<uint8_t> &v) { return std::string(v.begin(), v.end()); }

int main(int argc, char **argv) try {
    std::string adapter_name = argc > 1 ? argv[1] : "";
    std::string peer = argc > 2 ? argv[2] : "bluebus-c-e2e";

    bluebus::Session session;
    bluebus::Adapter adapter =
        adapter_name.empty() ? session.default_adapter() : session.adapter(adapter_name);
    adapter.power_on();
    std::printf("central on %s (%s), looking for \"%s\"\n", adapter.name().c_str(),
                adapter.address().c_str(), peer.c_str());

    bluebus::Device device =
        adapter.find_device(bluebus::ScanFilter().name(peer).transport(BLUEBUS_TRANSPORT_LE), 30000);
    EXPECT(true, ("found " + device.address()).c_str());
    device.connect(30000);
    EXPECT(device.is_connected(), "connected, services resolved");

    bool has_char = false;
    for (const auto &a : device.services()) {
        if (a.kind == BLUEBUS_ATTR_CHARACTERISTIC && a.uuid == CHAR_UUID) {
            has_char = (a.flags & BLUEBUS_FLAG_NOTIFY) && (a.flags & BLUEBUS_FLAG_WRITE);
        }
    }
    EXPECT(has_char, "service discovery lists the characteristic with its flags");
    EXPECT(text(device.read(CHAR_UUID)) == "hello", "read initial value \"hello\"");
    EXPECT(text(device.read_descriptor(CHAR_UUID, "2901")) == "echo", "read descriptor 2901");

    // Callback notifications (on a library thread). BlueZ also reports the
    // values this central writes or reads as changes, so wait for the echo.
    std::mutex m;
    std::condition_variable cv;
    std::vector<std::string> got;
    bluebus::Notifications sub = device.subscribe(CHAR_UUID, [&](const std::vector<uint8_t> &v) {
        std::lock_guard<std::mutex> lock(m);
        got.push_back(text(v));
        cv.notify_all();
    });
    device.write(CHAR_UUID, {'p', 'i', 'n', 'g'}, true);
    {
        std::unique_lock<std::mutex> lock(m);
        bool echoed = cv.wait_for(lock, std::chrono::seconds(10), [&] {
            for (const auto &g : got)
                if (g == "echo:ping") return true;
            return false;
        });
        EXPECT(echoed, "write request answered by notification");
    }
    sub.stop();

    // Pull notifications, write without response.
    bluebus::Notifications pull = device.notifications(CHAR_UUID);
    device.write(CHAR_UUID, {'c', 'm', 'd'}, false);
    bool echoed = false;
    for (int i = 0; i < 5 && !echoed; ++i) {
        auto v = pull.next(10000);
        echoed = v && text(*v) == "echo:cmd";
    }
    EXPECT(echoed, "write command answered (pull notifications)");
    pull.stop();

    session.set_agent_auto();
    device.pair();
    device.trust();
    auto info = device.info();
    EXPECT(info.paired, "paired (Just Works)");
    EXPECT(info.trusted, "trusted");
    session.remove_agent();

    device.write(CHAR_UUID, {'b', 'y', 'e'}, true);
    device.disconnect();
    EXPECT(!device.is_connected(), "disconnected");
    adapter.remove_device(device.address());
    EXPECT(true, "removed the pairing");

    std::printf("\nALL %d C++ CENTRAL CHECKS PASSED\n", checks);
    return 0;
} catch (const bluebus::Error &e) {
    std::fprintf(stderr, "FAILED: bluebus error %d (%s): %s\n", e.status(), e.dbus_name().c_str(),
                 e.what());
    return 1;
}
