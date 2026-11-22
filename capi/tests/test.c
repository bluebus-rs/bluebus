/* C test of libbluebus against the mock BlueZ of the Rust tests.
 *
 * Run through `cargo test -p bluebus-c --test c_programs` or CTest, which
 * start the mock and point DBUS_SYSTEM_BUS_ADDRESS at it. Exit status 0
 * means every check passed. */
#define _POSIX_C_SOURCE 199309L /* nanosleep */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "bluebus.h"

static int checks = 0;

#define CHECK(cond)                                                                   \
    do {                                                                              \
        ++checks;                                                                     \
        if (!(cond)) {                                                                \
            fprintf(stderr, "%s:%d: CHECK(%s) failed; last error: %s\n", __FILE__,    \
                    __LINE__, #cond, bluebus_last_error());                           \
            exit(1);                                                                  \
        }                                                                             \
    } while (0)

#define OK(call) CHECK((call) == BLUEBUS_OK)

static void sleep_ms(long ms) {
    struct timespec ts;
    ts.tv_sec = ms / 1000;
    ts.tv_nsec = (ms % 1000) * 1000000L;
    nanosleep(&ts, NULL);
}

/* Shared with the callbacks, which run on library threads. Plain ints
 * written by one thread and polled by another are enough for this test
 * (the library joins its threads on stop). */
static volatile int scan_found = 0;
static volatile int notified = 0;
static volatile uint8_t last_notified = 0;

static void on_scan(const bluebus_device_info *d, int kind, void *user) {
    (void)user;
    if (kind == BLUEBUS_SCAN_FOUND && d->address != NULL) ++scan_found;
}

static void on_notify(const uint8_t *data, size_t len, void *user) {
    CHECK(user == (void *)&notified);
    if (len == 1) last_notified = data[0];
    ++notified;
}

static int wait_for(volatile int *v, int at_least) {
    for (int i = 0; i < 500 && *v < at_least; ++i) sleep_ms(10);
    return *v >= at_least;
}

int main(void) {
    printf("bluebus %s (header %s)\n", bluebus_version(), BLUEBUS_VERSION_STRING);
    CHECK(strcmp(bluebus_version(), BLUEBUS_VERSION_STRING) == 0);

    char uuid[37];
    OK(bluebus_normalize_uuid("2A19", uuid, sizeof uuid));
    CHECK(strcmp(uuid, "00002a19-0000-1000-8000-00805f9b34fb") == 0);
    CHECK(bluebus_session_new(NULL) == BLUEBUS_ERR_INVALID_ARGUMENT);
    CHECK(strstr(bluebus_last_error(), "NULL") != NULL);

    bluebus_session *session = NULL;
    OK(bluebus_session_new(&session));

    bluebus_adapter_info *adapters = NULL;
    size_t adapter_count = 0;
    OK(bluebus_session_adapters(session, &adapters, &adapter_count));
    CHECK(adapter_count == 2);
    CHECK(strcmp(adapters[0].name, "hci0") == 0);
    bluebus_adapter_info_free(adapters, adapter_count);

    bluebus_adapter *missing = NULL;
    CHECK(bluebus_session_adapter(session, "hci7", &missing) == BLUEBUS_ERR_NOT_FOUND);
    CHECK(missing == NULL);

    bluebus_adapter *adapter = NULL;
    OK(bluebus_session_default_adapter(session, &adapter));
    CHECK(strcmp(bluebus_adapter_name(adapter), "hci0") == 0);

    /* Scanning an unpowered adapter: a named D-Bus error. */
    bluebus_device_info *devices = NULL;
    size_t count = 0;
    CHECK(bluebus_adapter_scan(adapter, NULL, 10, &devices, &count) == BLUEBUS_ERR_DBUS);
    CHECK(strcmp(bluebus_last_dbus_error_name(), "org.bluez.Error.NotReady") == 0);

    OK(bluebus_adapter_power_on(adapter));
    bool powered = false;
    OK(bluebus_adapter_is_powered(adapter, &powered));
    CHECK(powered);

    OK(bluebus_adapter_scan(adapter, NULL, 100, &devices, &count));
    CHECK(count == 2);
    CHECK(strcmp(devices[0].address, "11:22:33:44:55:66") == 0);
    CHECK(strcmp(devices[0].name, "Headset") == 0);
    CHECK(devices[0].has_rssi && devices[0].rssi == -48);
    CHECK(devices[1].name == NULL);
    bluebus_device_info_free(devices, count);

    bluebus_scan_filter filter = {0};
    filter.name_prefix = "Head";
    filter.transport = BLUEBUS_TRANSPORT_LE;
    OK(bluebus_adapter_scan(adapter, &filter, 100, &devices, &count));
    CHECK(count == 1);
    bluebus_device_info_free(devices, count);

    /* Callback scan. */
    bluebus_scan *scan = NULL;
    OK(bluebus_adapter_scan_start(adapter, NULL, on_scan, NULL, &scan));
    CHECK(wait_for(&scan_found, 2));
    OK(bluebus_scan_stop(scan));
    bluebus_scan_free(scan);

    /* Central: find, connect, GATT. */
    bluebus_scan_filter by_name = {0};
    by_name.name = "Headset";
    bluebus_device *device = NULL;
    OK(bluebus_adapter_find_device(adapter, &by_name, 5000, &device));
    CHECK(strcmp(bluebus_device_address(device), "11:22:33:44:55:66") == 0);
    OK(bluebus_device_connect(device, 5000));
    bool connected = false;
    OK(bluebus_device_is_connected(device, &connected));
    CHECK(connected);

    uint8_t *data = NULL;
    size_t len = 0;
    OK(bluebus_device_read(device, "2a19", &data, &len));
    CHECK(len == 1 && data[0] == 87);
    bluebus_bytes_free(data);
    CHECK(bluebus_device_read(device, "2a37", &data, &len) == BLUEBUS_ERR_NOT_FOUND);
    CHECK(data == NULL && len == 0);

    uint8_t level = 0;
    OK(bluebus_device_battery_level(device, &level));
    CHECK(level == 87);

    bluebus_gatt_attribute *attrs = NULL;
    size_t attr_count = 0;
    OK(bluebus_device_services(device, &attrs, &attr_count));
    CHECK(attr_count == 3);
    CHECK(attrs[0].kind == BLUEBUS_ATTR_SERVICE);
    CHECK(attrs[1].kind == BLUEBUS_ATTR_CHARACTERISTIC);
    CHECK(attrs[1].flags & BLUEBUS_FLAG_NOTIFY);
    CHECK(attrs[2].kind == BLUEBUS_ATTR_DESCRIPTOR);
    CHECK(strcmp(attrs[2].characteristic_uuid, "00002a19-0000-1000-8000-00805f9b34fb") == 0);
    bluebus_gatt_attributes_free(attrs, attr_count);

    /* Notifications: the mock notifies every value written. */
    bluebus_notifications *sub = NULL;
    OK(bluebus_device_subscribe(device, "2a19", on_notify, (void *)&notified, &sub));
    bluebus_notifications *pull = NULL;
    OK(bluebus_device_notifications(device, "2a19", &pull));
    CHECK(bluebus_notifications_next(pull, 20, &data, &len) == BLUEBUS_ERR_TIMEOUT);

    const uint8_t v42[] = {42};
    OK(bluebus_device_write(device, "2a19", v42, sizeof v42, true));
    CHECK(wait_for(&notified, 1));
    CHECK(last_notified == 42);
    OK(bluebus_notifications_next(pull, 2000, &data, &len));
    CHECK(len == 1 && data[0] == 42);
    bluebus_bytes_free(data);

    OK(bluebus_notifications_stop(sub));
    bluebus_notifications_free(sub);
    bluebus_notifications_free(pull);

    /* Pairing with the auto-accept agent. */
    OK(bluebus_session_set_agent_auto(session));
    OK(bluebus_device_pair(device));
    OK(bluebus_device_trust(device, true));
    bluebus_device_info *info = NULL;
    OK(bluebus_device_get_info(device, &info));
    CHECK(info->paired && info->trusted && info->connected);
    bluebus_device_info_free(info, 1);
    OK(bluebus_session_remove_agent(session));

    OK(bluebus_device_disconnect(device));
    bluebus_device_free(device);

    /* Peripheral. */
    bluebus_peripheral_builder *builder = NULL;
    OK(bluebus_peripheral_builder_new(adapter, &builder));
    OK(bluebus_peripheral_builder_add_service(builder, "180f", true));
    const uint8_t full[] = {100};
    OK(bluebus_peripheral_builder_add_characteristic(builder, "180f", "2a19",
                                                     BLUEBUS_FLAG_READ | BLUEBUS_FLAG_NOTIFY,
                                                     full, sizeof full, NULL, NULL));
    CHECK(bluebus_peripheral_builder_add_characteristic(builder, "1234", "2a00",
                                                        BLUEBUS_FLAG_READ, NULL, 0, NULL,
                                                        NULL) == BLUEBUS_ERR_NOT_FOUND);
    const char *const uuids[] = {"180f"};
    OK(bluebus_peripheral_builder_advertise(builder, "bluebus-c-test", uuids, 1));
    bluebus_peripheral *peripheral = NULL;
    OK(bluebus_peripheral_builder_start(builder, &peripheral));
    CHECK(bluebus_peripheral_is_running(peripheral));
    const uint8_t v90[] = {90};
    OK(bluebus_peripheral_notify(peripheral, "2a19", v90, sizeof v90));
    OK(bluebus_peripheral_value(peripheral, "2a19", &data, &len));
    CHECK(len == 1 && data[0] == 90);
    bluebus_bytes_free(data);
    bool notifying = true;
    OK(bluebus_peripheral_is_notifying(peripheral, "2a19", &notifying));
    CHECK(!notifying);
    OK(bluebus_peripheral_stop(peripheral));
    CHECK(!bluebus_peripheral_is_running(peripheral));
    bluebus_peripheral_free(peripheral);
    bluebus_peripheral_builder_free(builder);

    bluebus_adapter_free(adapter);
    bluebus_session_free(session);
    printf("all %d C checks passed\n", checks);
    return 0;
}
