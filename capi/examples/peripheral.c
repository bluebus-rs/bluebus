/* A BLE peripheral: one service with a read/write/notify characteristic.
 * Every value a central writes is answered with a notification
 * "echo:<value>"; writing "bye" ends the program.
 *
 *   cc peripheral.c -Iinclude -Ltarget/release -lbluebus -o peripheral
 *   ./peripheral [adapter] [seconds]          e.g. ./peripheral hci1 60
 *
 * Pair with it from the central (examples/central.cpp): it registers an
 * agent that accepts "Just Works" pairing.
 */
#define _POSIX_C_SOURCE 199309L /* nanosleep */
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "bluebus.h"

#define SERVICE_UUID "b1eb05e0-0000-4000-8000-00000000c0e0"
#define CHAR_UUID "b1eb05e0-0000-4000-8000-00000000c0e1"
#define LOCAL_NAME "bluebus-c-e2e"

static bluebus_peripheral *peripheral = NULL;
static volatile sig_atomic_t done = 0;
static volatile sig_atomic_t writes = 0;

static void on_signal(int sig) {
    (void)sig;
    done = 1;
}

/* Runs on the library's peripheral thread; may call bluebus functions. */
static void on_write(const uint8_t *data, size_t len, void *user) {
    (void)user;
    printf("central wrote %zu byte(s): %.*s\n", len, (int)len, (const char *)data);
    fflush(stdout);
    ++writes;
    if (len == 3 && memcmp(data, "bye", 3) == 0) {
        done = 1;
        return;
    }
    uint8_t reply[64] = "echo:";
    size_t n = len < sizeof reply - 5 ? len : sizeof reply - 5;
    memcpy(reply + 5, data, n);
    bool subscribed = false;
    if (bluebus_peripheral_is_notifying(peripheral, CHAR_UUID, &subscribed) != BLUEBUS_OK)
        fprintf(stderr, "is_notifying: %s\n", bluebus_last_error());
    if (bluebus_peripheral_notify(peripheral, CHAR_UUID, reply, n + 5) != BLUEBUS_OK)
        fprintf(stderr, "notify: %s\n", bluebus_last_error());
    printf("  notified \"%.*s\"%s\n", (int)(n + 5), (const char *)reply,
           subscribed ? "" : " (no central subscribed)");
    fflush(stdout);
}

/* Whether `address` is among the `count` devices of `list` and paired. */
static bool was_paired(const bluebus_device_info *list, size_t count, const char *address) {
    for (size_t i = 0; i < count; ++i)
        if (list[i].paired && strcmp(list[i].address, address) == 0) return true;
    return false;
}

/* Removes a central's pairing once it has disconnected (BlueZ may not
 * answer RemoveDevice while the link is still being torn down). */
static void forget(const bluebus_adapter *adapter, const char *address) {
    bluebus_device *device = NULL;
    if (bluebus_adapter_device(adapter, address, &device) != BLUEBUS_OK) return;
    bool connected = true;
    for (int i = 0; i < 100 && connected; ++i) {
        if (bluebus_device_is_connected(device, &connected) != BLUEBUS_OK) break;
        if (connected) {
            struct timespec ts = {0, 100 * 1000000L};
            nanosleep(&ts, NULL);
        }
    }
    bluebus_device_free(device);
    printf("forgetting %s\n", address);
    if (bluebus_adapter_remove_device(adapter, address) != BLUEBUS_OK)
        fprintf(stderr, "remove %s: %s\n", address, bluebus_last_error());
}

static int fail(const char *what) {
    fprintf(stderr, "%s: %s\n", what, bluebus_last_error());
    return 1;
}

int main(int argc, char **argv) {
    const char *name = argc > 1 ? argv[1] : NULL;
    long seconds = argc > 2 ? atol(argv[2]) : 60;
    setvbuf(stdout, NULL, _IOLBF, 0); /* line by line, also into a file */
    signal(SIGINT, on_signal);
    signal(SIGTERM, on_signal);

    bluebus_session *session = NULL;
    if (bluebus_session_new(&session) != BLUEBUS_OK) return fail("session");
    bluebus_adapter *adapter = NULL;
    if (bluebus_session_adapter(session, name, &adapter) != BLUEBUS_OK) return fail("adapter");
    if (bluebus_adapter_power_on(adapter) != BLUEBUS_OK) return fail("power on");
    if (bluebus_session_set_agent_auto(session) != BLUEBUS_OK) return fail("agent");
    /* Devices paired before this run (kept at exit, see below). */
    bluebus_device_info *before = NULL;
    size_t before_count = 0;
    if (bluebus_adapter_devices(adapter, &before, &before_count) != BLUEBUS_OK)
        return fail("devices");

    bluebus_peripheral_builder *builder = NULL;
    if (bluebus_peripheral_builder_new(adapter, &builder) != BLUEBUS_OK) return fail("builder");
    const char hello[] = "hello";
    const char *const adv_uuids[] = {SERVICE_UUID};
    if (bluebus_peripheral_builder_add_service(builder, SERVICE_UUID, true) != BLUEBUS_OK ||
        bluebus_peripheral_builder_add_characteristic(
            builder, SERVICE_UUID, CHAR_UUID,
            BLUEBUS_FLAG_READ | BLUEBUS_FLAG_WRITE | BLUEBUS_FLAG_WRITE_WITHOUT_RESPONSE |
                BLUEBUS_FLAG_NOTIFY,
            (const uint8_t *)hello, strlen(hello), on_write, NULL) != BLUEBUS_OK ||
        bluebus_peripheral_builder_add_descriptor(builder, CHAR_UUID, "2901",
                                                  (const uint8_t *)"echo", 4) != BLUEBUS_OK ||
        bluebus_peripheral_builder_advertise(builder, LOCAL_NAME, adv_uuids, 1) != BLUEBUS_OK)
        return fail("describe");
    if (bluebus_peripheral_builder_start(builder, &peripheral) != BLUEBUS_OK) return fail("start");
    printf("advertising \"%s\" on %s (%s)\n", LOCAL_NAME, bluebus_adapter_name(adapter),
           bluebus_adapter_address(adapter));
    fflush(stdout);

    for (long ms = 0; !done && ms < seconds * 1000; ms += 100) {
        struct timespec ts = {0, 100 * 1000000L};
        nanosleep(&ts, NULL);
    }

    uint8_t *value = NULL;
    size_t len = 0;
    if (bluebus_peripheral_value(peripheral, CHAR_UUID, &value, &len) == BLUEBUS_OK) {
        printf("final value: %.*s (%d write(s))\n", (int)len, (const char *)value, (int)writes);
        bluebus_bytes_free(value);
    }
    if (bluebus_peripheral_stop(peripheral) != BLUEBUS_OK) return fail("stop");
    bluebus_peripheral_free(peripheral);

    /* Forget the centrals that paired during this run, so that the next run
     * starts clean (the central forgets this peripheral too). */
    bluebus_device_info *now = NULL;
    size_t now_count = 0;
    if (bluebus_adapter_devices(adapter, &now, &now_count) == BLUEBUS_OK) {
        for (size_t i = 0; i < now_count; ++i) {
            if (now[i].paired && !was_paired(before, before_count, now[i].address)) {
                forget(adapter, now[i].address);
            }
        }
        bluebus_device_info_free(now, now_count);
    }
    bluebus_device_info_free(before, before_count);
    bluebus_peripheral_builder_free(builder);
    bluebus_session_remove_agent(session);
    bluebus_adapter_free(adapter);
    bluebus_session_free(session);
    printf("peripheral stopped\n");
    return writes > 0 ? 0 : 2;
}
