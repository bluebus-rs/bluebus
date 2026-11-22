/**
 * @file bluebus.h
 * @brief C API of bluebus: Bluetooth Low Energy on Linux through BlueZ.
 *
 * A blocking C interface over the high-level `bluebus::ble` Rust API:
 * adapters, scanning, connecting, pairing, GATT read / write /
 * notifications, and a local GATT server with an LE advertisement.
 *
 * @par Threads
 * Every function blocks the calling thread until BlueZ answered; the
 * library runs its own internal runtime (started on first use). All
 * functions and handles are thread-safe: a handle may be used from several
 * threads at once.
 *
 * @par Callbacks
 * Scan, notification and write callbacks run on threads owned by the
 * library (one per scan, subscription or peripheral), never on the thread
 * that registered them, so protect shared data. A callback may call any
 * bluebus function, including stopping or freeing its own handle (the
 * handle then stops after the callback returns). Pointers passed to a
 * callback are valid only during the call. Callbacks must not throw C++
 * exceptions or `longjmp` out.
 *
 * @par Errors
 * Fallible functions return a ::bluebus_status. On failure,
 * bluebus_last_error() describes the error and, for D-Bus errors,
 * bluebus_last_dbus_error_name() names it (for example
 * `"org.bluez.Error.NotReady"`). Both are per thread.
 *
 * @par Memory
 * Handles are released with their `_free` function (`NULL` is accepted).
 * Byte buffers returned by the library are released with
 * bluebus_bytes_free(), arrays with their matching `_free` function.
 * Strings returned by a handle getter (bluebus_adapter_name(), ...) live as
 * long as the handle. Output pointers are set to `NULL` / 0 on failure.
 *
 * @par UUIDs
 * UUIDs may be given in short (`"2a19"`, `"0x2A19"`) or full form
 * (`"00002a19-0000-1000-8000-00805f9b34fb"`). The library returns full,
 * lower-case UUIDs.
 */
#ifndef BLUEBUS_H
#define BLUEBUS_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/** @name Version of this header (see also bluebus_version()). */
/**@{*/
#define BLUEBUS_VERSION_MAJOR 0
#define BLUEBUS_VERSION_MINOR 1
#define BLUEBUS_VERSION_PATCH 0
#define BLUEBUS_VERSION_STRING "0.1.0"
/**@}*/

/** Pass as a timeout to wait without limit (bluebus_notifications_next()). */
#define BLUEBUS_WAIT_FOREVER UINT32_MAX

/** Result of a fallible function. */
typedef enum bluebus_status {
    BLUEBUS_OK = 0,                   /**< Success. */
    BLUEBUS_ERR_TIMEOUT = 1,          /**< The operation did not complete in time. */
    BLUEBUS_ERR_NOT_FOUND = 2,        /**< No such adapter, device, service, characteristic or descriptor. */
    BLUEBUS_ERR_INVALID_ARGUMENT = 3, /**< NULL or malformed argument (UUID, address, flags, ...). */
    BLUEBUS_ERR_DBUS = 4,             /**< D-Bus or BlueZ error; see bluebus_last_dbus_error_name(). */
    BLUEBUS_ERR_CLOSED = 5,           /**< The subscription ended or was stopped. */
    BLUEBUS_ERR_INTERNAL = 6          /**< Unexpected internal failure (a bug). */
} bluebus_status;

/** @name Opaque handles */
/**@{*/
typedef struct bluebus_session bluebus_session;       /**< Connection to BlueZ. */
typedef struct bluebus_adapter bluebus_adapter;       /**< Local adapter (hci0, ...). */
typedef struct bluebus_device bluebus_device;         /**< Remote device. */
typedef struct bluebus_scan bluebus_scan;             /**< Running callback scan. */
typedef struct bluebus_notifications bluebus_notifications; /**< Characteristic subscription. */
typedef struct bluebus_peripheral_builder bluebus_peripheral_builder; /**< GATT server under construction. */
typedef struct bluebus_peripheral bluebus_peripheral; /**< Running GATT server / advertisement. */
/**@}*/

/* ------------------------------------------------------------------------ */
/** @name Errors, version, memory */
/**@{*/

/** Message of the last failed call on this thread ("" if none). Valid until
 *  the next failing call on this thread. Never NULL. */
const char *bluebus_last_error(void);

/** D-Bus error name of the last failed call on this thread, or NULL if it
 *  was not a D-Bus error. Same lifetime as bluebus_last_error(). */
const char *bluebus_last_dbus_error_name(void);

/** Static description of a status ("timeout", ...). Never NULL. */
const char *bluebus_status_str(bluebus_status status);

/** Version of the library, e.g. "0.1.0" (static string). */
const char *bluebus_version(void);

/** Frees a byte buffer returned by the library. NULL is ignored. */
void bluebus_bytes_free(uint8_t *data);

/** Writes the full 128-bit form of @p uuid ("2a19" →
 *  "00002a19-0000-1000-8000-00805f9b34fb") into @p out_uuid, which must
 *  hold at least 37 bytes (@p size). */
bluebus_status bluebus_normalize_uuid(const char *uuid, char *out_uuid, size_t size);
/**@}*/

/* ------------------------------------------------------------------------ */
/** @name Plain data */
/**@{*/

/** Manufacturer-specific advertising data. */
typedef struct bluebus_manufacturer_data {
    uint16_t company_id;
    const uint8_t *data;
    size_t len;
} bluebus_manufacturer_data;

/** Service advertising data. */
typedef struct bluebus_service_data {
    const char *uuid;
    const uint8_t *data;
    size_t len;
} bluebus_service_data;

/** Snapshot of a remote device. Free arrays with bluebus_device_info_free(). */
typedef struct bluebus_device_info {
    const char *address;      /**< "AA:BB:CC:DD:EE:FF" */
    const char *address_type; /**< "public" / "random", or NULL if unknown */
    const char *name;         /**< Advertised name, or NULL */
    const char *alias;        /**< Name to display (BlueZ alias) */
    int16_t rssi;             /**< dBm, valid if has_rssi */
    int16_t tx_power;         /**< dBm, valid if has_tx_power */
    bool has_rssi;
    bool has_tx_power;
    bool connected;
    bool paired;
    bool trusted;
    bool services_resolved;
    const char *const *uuids; /**< Advertised / resolved service UUIDs */
    size_t uuid_count;
    const bluebus_manufacturer_data *manufacturer_data;
    size_t manufacturer_data_count;
    const bluebus_service_data *service_data;
    size_t service_data_count;
} bluebus_device_info;

/** Frees @p count device infos returned by the library (NULL is ignored). */
void bluebus_device_info_free(bluebus_device_info *infos, size_t count);

/** A local adapter. Free arrays with bluebus_adapter_info_free(). */
typedef struct bluebus_adapter_info {
    const char *name;    /**< "hci0" */
    const char *path;    /**< "/org/bluez/hci0" */
    const char *address; /**< MAC address */
    const char *alias;
    bool powered;
    bool discovering;
} bluebus_adapter_info;

/** Frees @p count adapter infos returned by the library. */
void bluebus_adapter_info_free(bluebus_adapter_info *infos, size_t count);

/** @name Characteristic / descriptor flags (bitmask) */
/**@{*/
#define BLUEBUS_FLAG_BROADCAST                   (1u << 0)
#define BLUEBUS_FLAG_READ                        (1u << 1)
#define BLUEBUS_FLAG_WRITE_WITHOUT_RESPONSE      (1u << 2)
#define BLUEBUS_FLAG_WRITE                       (1u << 3)
#define BLUEBUS_FLAG_NOTIFY                      (1u << 4)
#define BLUEBUS_FLAG_INDICATE                    (1u << 5)
#define BLUEBUS_FLAG_AUTHENTICATED_SIGNED_WRITES (1u << 6)
#define BLUEBUS_FLAG_EXTENDED_PROPERTIES         (1u << 7)
#define BLUEBUS_FLAG_RELIABLE_WRITE              (1u << 8)
#define BLUEBUS_FLAG_WRITABLE_AUXILIARIES        (1u << 9)
#define BLUEBUS_FLAG_ENCRYPT_READ                (1u << 10)
#define BLUEBUS_FLAG_ENCRYPT_WRITE               (1u << 11)
#define BLUEBUS_FLAG_ENCRYPT_NOTIFY              (1u << 12)
#define BLUEBUS_FLAG_ENCRYPT_INDICATE            (1u << 13)
#define BLUEBUS_FLAG_ENCRYPT_AUTHENTICATED_READ  (1u << 14)
#define BLUEBUS_FLAG_ENCRYPT_AUTHENTICATED_WRITE (1u << 15)
#define BLUEBUS_FLAG_SECURE_READ                 (1u << 16)
#define BLUEBUS_FLAG_SECURE_WRITE                (1u << 17)
#define BLUEBUS_FLAG_AUTHORIZE                   (1u << 18)
/**@}*/

/** Kind of a ::bluebus_gatt_attribute. */
typedef enum bluebus_attribute_kind {
    BLUEBUS_ATTR_SERVICE = 0,
    BLUEBUS_ATTR_CHARACTERISTIC = 1,
    BLUEBUS_ATTR_DESCRIPTOR = 2
} bluebus_attribute_kind;

/** One entry of the flattened GATT database of a device, in tree order:
 *  each service is followed by its characteristics, each characteristic by
 *  its descriptors. */
typedef struct bluebus_gatt_attribute {
    int kind;                        /**< ::bluebus_attribute_kind */
    const char *uuid;
    const char *path;                /**< D-Bus object path */
    const char *service_uuid;        /**< The service (itself for services) */
    const char *characteristic_uuid; /**< Owning characteristic of a descriptor, else NULL */
    uint32_t flags;                  /**< BLUEBUS_FLAG_* (0 for services) */
} bluebus_gatt_attribute;

/** Frees @p count attributes returned by bluebus_device_services(). */
void bluebus_gatt_attributes_free(bluebus_gatt_attribute *attributes, size_t count);
/**@}*/

/* ------------------------------------------------------------------------ */
/** @name Session and pairing agent */
/**@{*/

/** Connects to BlueZ on the D-Bus system bus (the address can be overridden
 *  with the `DBUS_SYSTEM_BUS_ADDRESS` environment variable). */
bluebus_status bluebus_session_new(bluebus_session **session);

/** Frees a session. Adapters and devices opened from it stay usable. */
void bluebus_session_free(bluebus_session *session);

/** Lists the local adapters sorted by name; free with bluebus_adapter_info_free(). */
bluebus_status bluebus_session_adapters(const bluebus_session *session,
                                        bluebus_adapter_info **adapters, size_t *count);

/** Opens an adapter by name ("hci1"), object path or MAC address. NULL
 *  selects the default adapter (hci0 if present, else the first one).
 *  ::BLUEBUS_ERR_NOT_FOUND if none matches. */
bluebus_status bluebus_session_adapter(const bluebus_session *session, const char *name,
                                       bluebus_adapter **adapter);

/** Same as bluebus_session_adapter(session, NULL, adapter). */
bluebus_status bluebus_session_default_adapter(const bluebus_session *session,
                                               bluebus_adapter **adapter);

/** Registers an agent that accepts every pairing confirmation ("Just Works",
 *  capability NoInputNoOutput) as BlueZ' default agent, replacing an agent
 *  set earlier on this session. It stays until removed or the session and
 *  everything opened from it is freed. */
bluebus_status bluebus_session_set_agent_auto(const bluebus_session *session);

/** Registers an agent answering passkey requests with @p passkey (0-999999)
 *  and accepting confirmations (capability KeyboardDisplay). */
bluebus_status bluebus_session_set_agent_passkey(const bluebus_session *session,
                                                 uint32_t passkey);

/** Unregisters the agent of this session (no-op if none). */
bluebus_status bluebus_session_remove_agent(const bluebus_session *session);
/**@}*/

/* ------------------------------------------------------------------------ */
/** @name Adapter and scanning */
/**@{*/

void bluebus_adapter_free(bluebus_adapter *adapter);

/** "hci0". Valid as long as the handle. */
const char *bluebus_adapter_name(const bluebus_adapter *adapter);
/** "/org/bluez/hci0". Valid as long as the handle. */
const char *bluebus_adapter_path(const bluebus_adapter *adapter);
/** MAC address. Valid as long as the handle. */
const char *bluebus_adapter_address(const bluebus_adapter *adapter);

/** Powers the adapter on and waits until BlueZ reports it. */
bluebus_status bluebus_adapter_power_on(const bluebus_adapter *adapter);
/** Powers the adapter off and waits until BlueZ reports it. */
bluebus_status bluebus_adapter_power_off(const bluebus_adapter *adapter);
bluebus_status bluebus_adapter_is_powered(const bluebus_adapter *adapter, bool *powered);

/** Transport of a ::bluebus_scan_filter. */
typedef enum bluebus_transport {
    BLUEBUS_TRANSPORT_DEFAULT = 0, /**< Not set: BlueZ default */
    BLUEBUS_TRANSPORT_AUTO = 1,    /**< LE and BR/EDR */
    BLUEBUS_TRANSPORT_BREDR = 2,   /**< Classic only */
    BLUEBUS_TRANSPORT_LE = 3       /**< Low Energy only */
} bluebus_transport;

/** Which devices a scan reports. Zero-initialise it (`= {0}`): every field
 *  left zero / NULL does not filter. A NULL filter pointer matches every
 *  device. Strings only need to live during the call. */
typedef struct bluebus_scan_filter {
    int transport;                     /**< ::bluebus_transport */
    const char *name;                  /**< Exact name, or NULL */
    const char *name_prefix;           /**< Name prefix, or NULL (not both with name) */
    const char *address;               /**< MAC address, or NULL */
    const char *const *service_uuids;  /**< Any of these advertised services */
    size_t service_uuid_count;
    int16_t min_rssi;                  /**< dBm, used if has_min_rssi */
    bool has_min_rssi;
    bool duplicate_data;               /**< Report every advertisement, used if has_duplicate_data */
    bool has_duplicate_data;
} bluebus_scan_filter;

/** Scans for @p duration_ms and returns the latest snapshot of every device
 *  that matched, sorted by address; free with bluebus_device_info_free().
 *  The adapter must be powered. */
bluebus_status bluebus_adapter_scan(const bluebus_adapter *adapter,
                                    const bluebus_scan_filter *filter, uint32_t duration_ms,
                                    bluebus_device_info **devices, size_t *count);

/** Kinds of scan callback events. */
typedef enum bluebus_scan_event {
    BLUEBUS_SCAN_FOUND = 0,  /**< A device matched for the first time in this scan */
    BLUEBUS_SCAN_UPDATED = 1 /**< A device already reported changed (RSSI, data, ...) */
} bluebus_scan_event;

/** Scan callback: @p device is valid during the call only; @p kind is a
 *  ::bluebus_scan_event. Runs on a library thread. */
typedef void (*bluebus_scan_callback)(const bluebus_device_info *device, int kind, void *user);

/** Starts a discovery that calls @p callback for each matching device until
 *  bluebus_scan_stop() or bluebus_scan_free(). One scan per adapter and
 *  session at a time. */
bluebus_status bluebus_adapter_scan_start(const bluebus_adapter *adapter,
                                          const bluebus_scan_filter *filter,
                                          bluebus_scan_callback callback, void *user,
                                          bluebus_scan **scan);

/** Stops the discovery and waits until no callback runs any more (unless
 *  called from the callback itself). Stopping twice is a no-op. */
bluebus_status bluebus_scan_stop(const bluebus_scan *scan);

/** Stops (if needed) and frees a scan. */
void bluebus_scan_free(bluebus_scan *scan);

/** Scans until a device matches @p filter (::BLUEBUS_ERR_TIMEOUT after
 *  @p timeout_ms). A connected device whose address is in the filter is
 *  returned without scanning. */
bluebus_status bluebus_adapter_find_device(const bluebus_adapter *adapter,
                                           const bluebus_scan_filter *filter,
                                           uint32_t timeout_ms, bluebus_device **device);

/** A handle to the device with this MAC address (not checked against BlueZ:
 *  operations on an unknown device fail with ::BLUEBUS_ERR_NOT_FOUND). */
bluebus_status bluebus_adapter_device(const bluebus_adapter *adapter, const char *address,
                                      bluebus_device **device);

/** Devices BlueZ knows on this adapter (paired, connected or recently seen). */
bluebus_status bluebus_adapter_devices(const bluebus_adapter *adapter,
                                       bluebus_device_info **devices, size_t *count);

/** Removes a device and its pairing information from BlueZ. Call it once
 *  the device is disconnected: BlueZ may not answer while the link is still
 *  being torn down, and this call has no timeout. */
bluebus_status bluebus_adapter_remove_device(const bluebus_adapter *adapter, const char *address);
/**@}*/

/* ------------------------------------------------------------------------ */
/** @name Remote device (central role) */
/**@{*/

void bluebus_device_free(bluebus_device *device);

/** "AA:BB:CC:DD:EE:FF". Valid as long as the handle. */
const char *bluebus_device_address(const bluebus_device *device);

/** Current snapshot; free with bluebus_device_info_free(info, 1). */
bluebus_status bluebus_device_get_info(const bluebus_device *device, bluebus_device_info **info);

/** Connects and waits until the GATT services are resolved (or
 *  ::BLUEBUS_ERR_TIMEOUT after @p timeout_ms). */
bluebus_status bluebus_device_connect(const bluebus_device *device, uint32_t timeout_ms);
bluebus_status bluebus_device_disconnect(const bluebus_device *device);
bluebus_status bluebus_device_is_connected(const bluebus_device *device, bool *connected);

/** Pairs, using the agent of the session (bluebus_session_set_agent_auto()). */
bluebus_status bluebus_device_pair(const bluebus_device *device);
/** Marks the device trusted (or not). */
bluebus_status bluebus_device_trust(const bluebus_device *device, bool trusted);

/** RSSI of the last advertisement; @p has_rssi is false if unknown. */
bluebus_status bluebus_device_rssi(const bluebus_device *device, int16_t *rssi, bool *has_rssi);

/** Battery level in percent (Battery Service). */
bluebus_status bluebus_device_battery_level(const bluebus_device *device, uint8_t *percent);

/** The GATT database, flattened; free with bluebus_gatt_attributes_free().
 *  The device must be connected. */
bluebus_status bluebus_device_services(const bluebus_device *device,
                                       bluebus_gatt_attribute **attributes, size_t *count);

/** Reads a characteristic; free @p data with bluebus_bytes_free(). An empty
 *  value gives data NULL and len 0. */
bluebus_status bluebus_device_read(const bluebus_device *device, const char *uuid,
                                   uint8_t **data, size_t *len);

/** Writes a characteristic, as a write request (@p with_response) or a write
 *  command (needs the write-without-response flag). */
bluebus_status bluebus_device_write(const bluebus_device *device, const char *uuid,
                                    const uint8_t *data, size_t len, bool with_response);

/** Reads a descriptor of a characteristic; free @p data with bluebus_bytes_free(). */
bluebus_status bluebus_device_read_descriptor(const bluebus_device *device,
                                              const char *characteristic_uuid,
                                              const char *descriptor_uuid, uint8_t **data,
                                              size_t *len);

/** Writes a descriptor of a characteristic. */
bluebus_status bluebus_device_write_descriptor(const bluebus_device *device,
                                               const char *characteristic_uuid,
                                               const char *descriptor_uuid,
                                               const uint8_t *data, size_t len);

/** Notification / indication callback: @p data is valid during the call
 *  only. Runs on a library thread. */
typedef void (*bluebus_notify_callback)(const uint8_t *data, size_t len, void *user);

/** Enables notifications of a characteristic and calls @p callback with
 *  each value until bluebus_notifications_stop() / _free(). Several
 *  subscriptions to one characteristic share it. */
bluebus_status bluebus_device_subscribe(const bluebus_device *device, const char *uuid,
                                        bluebus_notify_callback callback, void *user,
                                        bluebus_notifications **notifications);

/** Enables notifications of a characteristic; read the values with
 *  bluebus_notifications_next(). */
bluebus_status bluebus_device_notifications(const bluebus_device *device, const char *uuid,
                                            bluebus_notifications **notifications);

/** Waits up to @p timeout_ms (or ::BLUEBUS_WAIT_FOREVER) for the next value
 *  of a handle from bluebus_device_notifications(); free @p data with
 *  bluebus_bytes_free(). Values arriving while nobody waits are queued.
 *  ::BLUEBUS_ERR_TIMEOUT if none came, ::BLUEBUS_ERR_CLOSED once the
 *  subscription was stopped or the D-Bus connection closed. A disconnect
 *  does not end it (no more values arrive); watch
 *  bluebus_device_is_connected() and subscribe again after reconnecting. */
bluebus_status bluebus_notifications_next(const bluebus_notifications *notifications,
                                          uint32_t timeout_ms, uint8_t **data, size_t *len);

/** Disables notifications (unless another subscription still uses them) and
 *  waits for BlueZ. For callback subscriptions, waits until no callback
 *  runs any more (unless called from the callback). */
bluebus_status bluebus_notifications_stop(const bluebus_notifications *notifications);

/** Stops (if needed) and frees a subscription. */
void bluebus_notifications_free(bluebus_notifications *notifications);
/**@}*/

/* ------------------------------------------------------------------------ */
/** @name Local GATT server and advertisement (peripheral role) */
/**@{*/

/** Write callback of a local characteristic: called with every value a
 *  remote central wrote. Runs on a library thread (one per peripheral,
 *  calls are serialised). @p data is valid during the call only. */
typedef void (*bluebus_write_callback)(const uint8_t *data, size_t len, void *user);

/** Starts describing a peripheral on @p adapter. */
bluebus_status bluebus_peripheral_builder_new(const bluebus_adapter *adapter,
                                              bluebus_peripheral_builder **builder);
void bluebus_peripheral_builder_free(bluebus_peripheral_builder *builder);

/** Adds a GATT service (usually @p primary). */
bluebus_status bluebus_peripheral_builder_add_service(const bluebus_peripheral_builder *builder,
                                                      const char *uuid, bool primary);

/** Adds a characteristic to the service @p service_uuid (added before),
 *  with BLUEBUS_FLAG_* @p flags (at least one), an initial value and an
 *  optional write callback (NULL for none). */
bluebus_status bluebus_peripheral_builder_add_characteristic(
    const bluebus_peripheral_builder *builder, const char *service_uuid, const char *uuid,
    uint32_t flags, const uint8_t *value, size_t value_len, bluebus_write_callback on_write,
    void *user);

/** Adds a read-only descriptor (e.g. "2901", user description) to a
 *  characteristic added before. */
bluebus_status bluebus_peripheral_builder_add_descriptor(
    const bluebus_peripheral_builder *builder, const char *characteristic_uuid,
    const char *uuid, const uint8_t *value, size_t value_len);

/** Advertises while the peripheral runs: connectable and discoverable, with
 *  an optional @p local_name and service UUIDs. Replaces an earlier call. */
bluebus_status bluebus_peripheral_builder_advertise(const bluebus_peripheral_builder *builder,
                                                    const char *local_name,
                                                    const char *const *service_uuids,
                                                    size_t service_uuid_count);

/** Adds manufacturer data to the advertisement (creating it if needed). */
bluebus_status bluebus_peripheral_builder_advertise_manufacturer_data(
    const bluebus_peripheral_builder *builder, uint16_t company_id, const uint8_t *data,
    size_t len);

/** Registers the GATT application and the advertisement with BlueZ. The
 *  builder stays usable (and must still be freed). Nothing stays registered
 *  on error. */
bluebus_status bluebus_peripheral_builder_start(const bluebus_peripheral_builder *builder,
                                                bluebus_peripheral **peripheral);

/** Replaces the value of a local characteristic without notifying. */
bluebus_status bluebus_peripheral_set_value(const bluebus_peripheral *peripheral,
                                            const char *uuid, const uint8_t *data, size_t len);

/** Replaces the value and notifies (or indicates) subscribed centrals. */
bluebus_status bluebus_peripheral_notify(const bluebus_peripheral *peripheral, const char *uuid,
                                         const uint8_t *data, size_t len);

/** Current value of a local characteristic (after remote writes); free with
 *  bluebus_bytes_free(). */
bluebus_status bluebus_peripheral_value(const bluebus_peripheral *peripheral, const char *uuid,
                                        uint8_t **data, size_t *len);

/** Whether a central subscribed to a local characteristic. */
bluebus_status bluebus_peripheral_is_notifying(const bluebus_peripheral *peripheral,
                                               const char *uuid, bool *notifying);

/** Whether the peripheral is registered (not stopped). */
bool bluebus_peripheral_is_running(const bluebus_peripheral *peripheral);

/** Unregisters the advertisement and GATT application. Stopping twice is a
 *  no-op. */
bluebus_status bluebus_peripheral_stop(const bluebus_peripheral *peripheral);

/** Stops (if needed) and frees a peripheral; waits until no write callback
 *  runs any more (unless called from one). */
void bluebus_peripheral_free(bluebus_peripheral *peripheral);
/**@}*/

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* BLUEBUS_H */
