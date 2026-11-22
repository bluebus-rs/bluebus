/* Scans for BLE devices and prints them as they are found.
 *
 *   cc scan.c -Iinclude -Ltarget/release -lbluebus -o scan
 *   ./scan [adapter] [duration_ms] [name prefix]     e.g. ./scan hci0 5000 Sensor
 */
#define _POSIX_C_SOURCE 199309L /* nanosleep */
#include <stdio.h>
#include <stdlib.h>
#include <time.h>

#include "bluebus.h"

static void on_device(const bluebus_device_info *d, int kind, void *user) {
    (void)user;
    /* Runs on a library thread. */
    printf("%s %s", kind == BLUEBUS_SCAN_FOUND ? "found  " : "updated", d->address);
    if (d->name) printf("  name=%s", d->name);
    if (d->has_rssi) printf("  rssi=%d", d->rssi);
    for (size_t i = 0; i < d->manufacturer_data_count; ++i)
        printf("  mfr=0x%04x(%zu bytes)", d->manufacturer_data[i].company_id,
               d->manufacturer_data[i].len);
    printf("\n");
    fflush(stdout);
}

static int fail(const char *what) {
    fprintf(stderr, "%s: %s\n", what, bluebus_last_error());
    return 1;
}

int main(int argc, char **argv) {
    const char *name = argc > 1 ? argv[1] : NULL; /* NULL: default adapter */
    long duration_ms = argc > 2 ? atol(argv[2]) : 5000;

    bluebus_session *session = NULL;
    if (bluebus_session_new(&session) != BLUEBUS_OK) return fail("session");
    bluebus_adapter *adapter = NULL;
    if (bluebus_session_adapter(session, name, &adapter) != BLUEBUS_OK) return fail("adapter");
    if (bluebus_adapter_power_on(adapter) != BLUEBUS_OK) return fail("power on");
    printf("scanning on %s (%s) for %ld ms\n", bluebus_adapter_name(adapter),
           bluebus_adapter_address(adapter), duration_ms);

    bluebus_scan_filter filter = {0}; /* LE devices, optionally by name prefix */
    filter.transport = BLUEBUS_TRANSPORT_LE;
    if (argc > 3) filter.name_prefix = argv[3];
    bluebus_scan *scan = NULL;
    if (bluebus_adapter_scan_start(adapter, &filter, on_device, NULL, &scan) != BLUEBUS_OK)
        return fail("scan");
    struct timespec ts = {duration_ms / 1000, (duration_ms % 1000) * 1000000L};
    nanosleep(&ts, NULL);
    if (bluebus_scan_stop(scan) != BLUEBUS_OK) return fail("stop");
    bluebus_scan_free(scan);

    /* Everything BlueZ now knows on this adapter. */
    bluebus_device_info *devices = NULL;
    size_t count = 0;
    if (bluebus_adapter_devices(adapter, &devices, &count) != BLUEBUS_OK) return fail("devices");
    printf("%zu known device(s)\n", count);
    for (size_t i = 0; i < count; ++i)
        printf("  %s %s\n", devices[i].address, devices[i].alias);
    bluebus_device_info_free(devices, count);

    bluebus_adapter_free(adapter);
    bluebus_session_free(session);
    return 0;
}
