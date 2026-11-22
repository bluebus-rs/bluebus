/**
 * @file bluebus.hpp
 * @brief Header-only C++17 wrapper of bluebus.h.
 *
 * RAII classes (move-only) over the C handles, `std::string` /
 * `std::vector<uint8_t>` values, `std::function` callbacks and a
 * bluebus::Error exception for every failed call.
 *
 * @code
 * bluebus::Session session;
 * auto adapter = session.default_adapter();
 * adapter.power_on();
 * auto device = adapter.find_device(bluebus::ScanFilter().name_prefix("Sensor"), 10000);
 * device.connect(20000);
 * std::vector<uint8_t> level = device.read("2a19");
 * auto sub = device.subscribe("2a37", [](const std::vector<uint8_t>& v) { ... });
 * @endcode
 *
 * Callbacks run on library threads (see bluebus.h). An exception thrown by
 * a callback is caught and ignored. Do not destroy the object owning a
 * callback (Scan, Notifications, Peripheral) from inside that callback.
 */
#ifndef BLUEBUS_HPP
#define BLUEBUS_HPP

#include <cstdint>
#include <functional>
#include <map>
#include <memory>
#include <optional>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

#include "bluebus.h"

namespace bluebus {

/** A failed call: the status, bluebus_last_error() and the D-Bus error name. */
class Error : public std::runtime_error {
public:
    Error(bluebus_status status, const std::string &message, std::string dbus_name = {})
        : std::runtime_error(message), status_(status), dbus_name_(std::move(dbus_name)) {}

    /** The ::bluebus_status of the failed call. */
    bluebus_status status() const noexcept { return status_; }
    /** D-Bus error name such as "org.bluez.Error.NotReady", or "". */
    const std::string &dbus_name() const noexcept { return dbus_name_; }
    bool is_timeout() const noexcept { return status_ == BLUEBUS_ERR_TIMEOUT; }
    bool is_not_found() const noexcept { return status_ == BLUEBUS_ERR_NOT_FOUND; }

private:
    bluebus_status status_;
    std::string dbus_name_;
};

namespace detail {

inline void check(bluebus_status status) {
    if (status == BLUEBUS_OK) return;
    const char *name = bluebus_last_dbus_error_name();
    throw Error(status, bluebus_last_error(), name ? name : "");
}

inline std::string str(const char *s) { return s ? std::string(s) : std::string(); }

inline std::optional<std::string> opt_str(const char *s) {
    return s ? std::optional<std::string>(s) : std::nullopt;
}

inline std::vector<uint8_t> bytes(const uint8_t *data, size_t len) {
    return data ? std::vector<uint8_t>(data, data + len) : std::vector<uint8_t>();
}

/** Takes ownership of a byte buffer returned by the library. */
inline std::vector<uint8_t> take_bytes(uint8_t *data, size_t len) {
    std::vector<uint8_t> v = bytes(data, len);
    bluebus_bytes_free(data);
    return v;
}

template <typename T, void (*Free)(T *)> struct Deleter {
    void operator()(T *p) const noexcept { Free(p); }
};

template <typename T, void (*Free)(T *)> using Handle = std::unique_ptr<T, Deleter<T, Free>>;

inline std::vector<const char *> c_strs(const std::vector<std::string> &v) {
    std::vector<const char *> out;
    out.reserve(v.size());
    for (const auto &s : v) out.push_back(s.c_str());
    return out;
}

}  // namespace detail

/** Snapshot of a remote device (see ::bluebus_device_info). */
struct DeviceInfo {
    std::string address;
    std::optional<std::string> address_type;
    std::optional<std::string> name;
    std::string alias;
    std::optional<int16_t> rssi;
    std::optional<int16_t> tx_power;
    std::vector<std::string> uuids;
    std::map<uint16_t, std::vector<uint8_t>> manufacturer_data;
    std::map<std::string, std::vector<uint8_t>> service_data;
    bool connected = false;
    bool paired = false;
    bool trusted = false;
    bool services_resolved = false;

    static DeviceInfo from(const bluebus_device_info &d) {
        DeviceInfo i;
        i.address = detail::str(d.address);
        i.address_type = detail::opt_str(d.address_type);
        i.name = detail::opt_str(d.name);
        i.alias = detail::str(d.alias);
        if (d.has_rssi) i.rssi = d.rssi;
        if (d.has_tx_power) i.tx_power = d.tx_power;
        for (size_t k = 0; k < d.uuid_count; ++k) i.uuids.emplace_back(d.uuids[k]);
        for (size_t k = 0; k < d.manufacturer_data_count; ++k) {
            const auto &m = d.manufacturer_data[k];
            i.manufacturer_data[m.company_id] = detail::bytes(m.data, m.len);
        }
        for (size_t k = 0; k < d.service_data_count; ++k) {
            const auto &s = d.service_data[k];
            i.service_data[s.uuid] = detail::bytes(s.data, s.len);
        }
        i.connected = d.connected;
        i.paired = d.paired;
        i.trusted = d.trusted;
        i.services_resolved = d.services_resolved;
        return i;
    }

    /** Converts and frees an array returned by the library. */
    static std::vector<DeviceInfo> take(bluebus_device_info *infos, size_t count) {
        std::vector<DeviceInfo> out;
        try {
            for (size_t k = 0; k < count; ++k) out.push_back(from(infos[k]));
        } catch (...) {
            bluebus_device_info_free(infos, count);
            throw;
        }
        bluebus_device_info_free(infos, count);
        return out;
    }
};

/** A local adapter (see ::bluebus_adapter_info). */
struct AdapterInfo {
    std::string name, path, address, alias;
    bool powered = false;
    bool discovering = false;
};

/** One entry of the flattened GATT database (see ::bluebus_gatt_attribute). */
struct GattAttribute {
    bluebus_attribute_kind kind = BLUEBUS_ATTR_SERVICE;
    std::string uuid, path, service_uuid;
    std::optional<std::string> characteristic_uuid;
    uint32_t flags = 0;
};

/** Builder for ::bluebus_scan_filter. */
class ScanFilter {
public:
    ScanFilter &transport(bluebus_transport t) { transport_ = t; return *this; }
    ScanFilter &name(std::string n) { name_ = std::move(n); prefix_.reset(); return *this; }
    ScanFilter &name_prefix(std::string p) { prefix_ = std::move(p); name_.reset(); return *this; }
    ScanFilter &address(std::string a) { address_ = std::move(a); return *this; }
    ScanFilter &service_uuid(std::string u) { uuids_.push_back(std::move(u)); return *this; }
    ScanFilter &min_rssi(int16_t r) { min_rssi_ = r; return *this; }
    ScanFilter &duplicate_data(bool on) { duplicate_data_ = on; return *this; }

    /** Calls @p f with the C filter (valid during the call). */
    template <typename F> auto with_c(F &&f) const {
        std::vector<const char *> uuids = detail::c_strs(uuids_);
        bluebus_scan_filter c{};
        c.transport = transport_;
        c.name = name_ ? name_->c_str() : nullptr;
        c.name_prefix = prefix_ ? prefix_->c_str() : nullptr;
        c.address = address_ ? address_->c_str() : nullptr;
        c.service_uuids = uuids.empty() ? nullptr : uuids.data();
        c.service_uuid_count = uuids.size();
        c.has_min_rssi = min_rssi_.has_value();
        c.min_rssi = min_rssi_.value_or(0);
        c.has_duplicate_data = duplicate_data_.has_value();
        c.duplicate_data = duplicate_data_.value_or(false);
        return f(&c);
    }

private:
    bluebus_transport transport_ = BLUEBUS_TRANSPORT_DEFAULT;
    std::optional<std::string> name_, prefix_, address_;
    std::vector<std::string> uuids_;
    std::optional<int16_t> min_rssi_;
    std::optional<bool> duplicate_data_;
};

/** Scan callback event kind. */
enum class ScanEvent { Found = BLUEBUS_SCAN_FOUND, Updated = BLUEBUS_SCAN_UPDATED };

/** A running callback scan; stops when destroyed. */
class Scan {
public:
    using Callback = std::function<void(const DeviceInfo &, ScanEvent)>;

    Scan(Scan &&) noexcept = default;
    /** Stops this object's handle before taking over @p o. */
    Scan &operator=(Scan &&o) noexcept {
        handle_.reset();
        callback_ = std::move(o.callback_);
        handle_ = std::move(o.handle_);
        return *this;
    }

    /** Stops the discovery and waits for running callbacks. */
    void stop() { detail::check(bluebus_scan_stop(handle_.get())); }
    bluebus_scan *get() const noexcept { return handle_.get(); }

private:
    friend class Adapter;
    Scan() = default;
    static void trampoline(const bluebus_device_info *d, int kind, void *user) noexcept {
        try {
            (*static_cast<Callback *>(user))(DeviceInfo::from(*d), static_cast<ScanEvent>(kind));
        } catch (...) {
        }
    }
    // Declared first: destroyed after the handle (whose free joins the thread).
    std::unique_ptr<Callback> callback_;
    detail::Handle<bluebus_scan, bluebus_scan_free> handle_;
};

/** A subscription to a characteristic; stops when destroyed. */
class Notifications {
public:
    using Callback = std::function<void(const std::vector<uint8_t> &)>;

    Notifications(Notifications &&) noexcept = default;
    /** Stops this object's handle before taking over @p o. */
    Notifications &operator=(Notifications &&o) noexcept {
        handle_.reset();
        callback_ = std::move(o.callback_);
        handle_ = std::move(o.handle_);
        return *this;
    }

    /** Next value of a pull subscription (Device::notifications()), or
     *  std::nullopt after @p timeout_ms. Throws once the subscription ended
     *  (status ::BLUEBUS_ERR_CLOSED). */
    std::optional<std::vector<uint8_t>> next(uint32_t timeout_ms = BLUEBUS_WAIT_FOREVER) {
        uint8_t *data = nullptr;
        size_t len = 0;
        bluebus_status s = bluebus_notifications_next(handle_.get(), timeout_ms, &data, &len);
        if (s == BLUEBUS_ERR_TIMEOUT) return std::nullopt;
        detail::check(s);
        return detail::take_bytes(data, len);
    }
    void stop() { detail::check(bluebus_notifications_stop(handle_.get())); }
    bluebus_notifications *get() const noexcept { return handle_.get(); }

private:
    friend class Device;
    Notifications() = default;
    static void trampoline(const uint8_t *data, size_t len, void *user) noexcept {
        try {
            (*static_cast<Callback *>(user))(detail::bytes(data, len));
        } catch (...) {
        }
    }
    std::unique_ptr<Callback> callback_;
    detail::Handle<bluebus_notifications, bluebus_notifications_free> handle_;
};

/** A remote device. */
class Device {
public:
    explicit Device(bluebus_device *raw) : handle_(raw) {}

    std::string address() const { return detail::str(bluebus_device_address(handle_.get())); }

    DeviceInfo info() const {
        bluebus_device_info *info = nullptr;
        detail::check(bluebus_device_get_info(handle_.get(), &info));
        return DeviceInfo::take(info, 1).at(0);
    }
    /** Connects and waits for service discovery. */
    void connect(uint32_t timeout_ms = 30000) const {
        detail::check(bluebus_device_connect(handle_.get(), timeout_ms));
    }
    void disconnect() const { detail::check(bluebus_device_disconnect(handle_.get())); }
    bool is_connected() const {
        bool c = false;
        detail::check(bluebus_device_is_connected(handle_.get(), &c));
        return c;
    }
    void pair() const { detail::check(bluebus_device_pair(handle_.get())); }
    void trust(bool trusted = true) const {
        detail::check(bluebus_device_trust(handle_.get(), trusted));
    }
    std::optional<int16_t> rssi() const {
        int16_t r = 0;
        bool has = false;
        detail::check(bluebus_device_rssi(handle_.get(), &r, &has));
        return has ? std::optional<int16_t>(r) : std::nullopt;
    }
    uint8_t battery_level() const {
        uint8_t p = 0;
        detail::check(bluebus_device_battery_level(handle_.get(), &p));
        return p;
    }
    std::vector<GattAttribute> services() const {
        bluebus_gatt_attribute *attrs = nullptr;
        size_t n = 0;
        detail::check(bluebus_device_services(handle_.get(), &attrs, &n));
        std::vector<GattAttribute> out;
        try {
            for (size_t k = 0; k < n; ++k) {
                GattAttribute a;
                a.kind = static_cast<bluebus_attribute_kind>(attrs[k].kind);
                a.uuid = detail::str(attrs[k].uuid);
                a.path = detail::str(attrs[k].path);
                a.service_uuid = detail::str(attrs[k].service_uuid);
                a.characteristic_uuid = detail::opt_str(attrs[k].characteristic_uuid);
                a.flags = attrs[k].flags;
                out.push_back(std::move(a));
            }
        } catch (...) {
            bluebus_gatt_attributes_free(attrs, n);
            throw;
        }
        bluebus_gatt_attributes_free(attrs, n);
        return out;
    }
    std::vector<uint8_t> read(const std::string &uuid) const {
        uint8_t *data = nullptr;
        size_t len = 0;
        detail::check(bluebus_device_read(handle_.get(), uuid.c_str(), &data, &len));
        return detail::take_bytes(data, len);
    }
    void write(const std::string &uuid, const std::vector<uint8_t> &value,
               bool with_response = true) const {
        detail::check(bluebus_device_write(handle_.get(), uuid.c_str(), value.data(),
                                           value.size(), with_response));
    }
    std::vector<uint8_t> read_descriptor(const std::string &chr, const std::string &desc) const {
        uint8_t *data = nullptr;
        size_t len = 0;
        detail::check(bluebus_device_read_descriptor(handle_.get(), chr.c_str(), desc.c_str(),
                                                     &data, &len));
        return detail::take_bytes(data, len);
    }
    void write_descriptor(const std::string &chr, const std::string &desc,
                          const std::vector<uint8_t> &value) const {
        detail::check(bluebus_device_write_descriptor(handle_.get(), chr.c_str(), desc.c_str(),
                                                      value.data(), value.size()));
    }
    /** Calls @p callback (on a library thread) with every notified value. */
    Notifications subscribe(const std::string &uuid, Notifications::Callback callback) const {
        Notifications n;
        n.callback_ = std::make_unique<Notifications::Callback>(std::move(callback));
        bluebus_notifications *raw = nullptr;
        detail::check(bluebus_device_subscribe(handle_.get(), uuid.c_str(),
                                               &Notifications::trampoline, n.callback_.get(),
                                               &raw));
        n.handle_.reset(raw);
        return n;
    }
    /** A pull subscription: read values with Notifications::next(). */
    Notifications notifications(const std::string &uuid) const {
        Notifications n;
        bluebus_notifications *raw = nullptr;
        detail::check(bluebus_device_notifications(handle_.get(), uuid.c_str(), &raw));
        n.handle_.reset(raw);
        return n;
    }
    bluebus_device *get() const noexcept { return handle_.get(); }

private:
    detail::Handle<bluebus_device, bluebus_device_free> handle_;
};

class PeripheralBuilder;

/** A running GATT server / advertisement; stops when destroyed. */
class Peripheral {
public:
    Peripheral(Peripheral &&) noexcept = default;
    /** Stops this object's handle before taking over @p o. */
    Peripheral &operator=(Peripheral &&o) noexcept {
        handle_.reset();
        callbacks_ = std::move(o.callbacks_);
        handle_ = std::move(o.handle_);
        return *this;
    }

    void set_value(const std::string &uuid, const std::vector<uint8_t> &value) const {
        detail::check(
            bluebus_peripheral_set_value(handle_.get(), uuid.c_str(), value.data(), value.size()));
    }
    void notify(const std::string &uuid, const std::vector<uint8_t> &value) const {
        detail::check(
            bluebus_peripheral_notify(handle_.get(), uuid.c_str(), value.data(), value.size()));
    }
    std::vector<uint8_t> value(const std::string &uuid) const {
        uint8_t *data = nullptr;
        size_t len = 0;
        detail::check(bluebus_peripheral_value(handle_.get(), uuid.c_str(), &data, &len));
        return detail::take_bytes(data, len);
    }
    bool is_notifying(const std::string &uuid) const {
        bool n = false;
        detail::check(bluebus_peripheral_is_notifying(handle_.get(), uuid.c_str(), &n));
        return n;
    }
    bool is_running() const { return bluebus_peripheral_is_running(handle_.get()); }
    void stop() const { detail::check(bluebus_peripheral_stop(handle_.get())); }
    bluebus_peripheral *get() const noexcept { return handle_.get(); }

private:
    friend class PeripheralBuilder;
    Peripheral() = default;
    using WriteCallback = std::function<void(const std::vector<uint8_t> &)>;
    std::vector<std::shared_ptr<WriteCallback>> callbacks_;
    detail::Handle<bluebus_peripheral, bluebus_peripheral_free> handle_;
};

/** Describes a peripheral; start() publishes it. */
class PeripheralBuilder {
public:
    using WriteCallback = std::function<void(const std::vector<uint8_t> &)>;

    explicit PeripheralBuilder(bluebus_peripheral_builder *raw) : handle_(raw) {}

    PeripheralBuilder &service(const std::string &uuid, bool primary = true) {
        detail::check(bluebus_peripheral_builder_add_service(handle_.get(), uuid.c_str(), primary));
        return *this;
    }
    /** Adds a characteristic (BLUEBUS_FLAG_* @p flags) to a service added before. */
    PeripheralBuilder &characteristic(const std::string &service_uuid, const std::string &uuid,
                                      uint32_t flags, const std::vector<uint8_t> &value = {},
                                      WriteCallback on_write = nullptr) {
        std::shared_ptr<WriteCallback> cb;
        if (on_write) cb = std::make_shared<WriteCallback>(std::move(on_write));
        detail::check(bluebus_peripheral_builder_add_characteristic(
            handle_.get(), service_uuid.c_str(), uuid.c_str(), flags, value.data(), value.size(),
            cb ? &PeripheralBuilder::trampoline : nullptr, cb.get()));
        if (cb) callbacks_.push_back(std::move(cb));
        return *this;
    }
    PeripheralBuilder &descriptor(const std::string &chr_uuid, const std::string &uuid,
                                  const std::vector<uint8_t> &value) {
        detail::check(bluebus_peripheral_builder_add_descriptor(
            handle_.get(), chr_uuid.c_str(), uuid.c_str(), value.data(), value.size()));
        return *this;
    }
    PeripheralBuilder &advertise(const std::string &local_name,
                                 const std::vector<std::string> &service_uuids = {}) {
        std::vector<const char *> uuids = detail::c_strs(service_uuids);
        detail::check(bluebus_peripheral_builder_advertise(
            handle_.get(), local_name.empty() ? nullptr : local_name.c_str(),
            uuids.empty() ? nullptr : uuids.data(), uuids.size()));
        return *this;
    }
    PeripheralBuilder &manufacturer_data(uint16_t company_id, const std::vector<uint8_t> &data) {
        detail::check(bluebus_peripheral_builder_advertise_manufacturer_data(
            handle_.get(), company_id, data.data(), data.size()));
        return *this;
    }
    Peripheral start() const {
        Peripheral p;
        p.callbacks_ = callbacks_;
        bluebus_peripheral *raw = nullptr;
        detail::check(bluebus_peripheral_builder_start(handle_.get(), &raw));
        p.handle_.reset(raw);
        return p;
    }

private:
    static void trampoline(const uint8_t *data, size_t len, void *user) noexcept {
        try {
            (*static_cast<WriteCallback *>(user))(detail::bytes(data, len));
        } catch (...) {
        }
    }
    std::vector<std::shared_ptr<WriteCallback>> callbacks_;
    detail::Handle<bluebus_peripheral_builder, bluebus_peripheral_builder_free> handle_;
};

/** A local adapter. */
class Adapter {
public:
    explicit Adapter(bluebus_adapter *raw) : handle_(raw) {}

    std::string name() const { return detail::str(bluebus_adapter_name(handle_.get())); }
    std::string path() const { return detail::str(bluebus_adapter_path(handle_.get())); }
    std::string address() const { return detail::str(bluebus_adapter_address(handle_.get())); }
    void power_on() const { detail::check(bluebus_adapter_power_on(handle_.get())); }
    void power_off() const { detail::check(bluebus_adapter_power_off(handle_.get())); }
    bool is_powered() const {
        bool p = false;
        detail::check(bluebus_adapter_is_powered(handle_.get(), &p));
        return p;
    }
    /** Scans for @p duration_ms and returns the matching devices. */
    std::vector<DeviceInfo> scan(const ScanFilter &filter, uint32_t duration_ms) const {
        bluebus_device_info *infos = nullptr;
        size_t n = 0;
        filter.with_c([&](const bluebus_scan_filter *f) {
            detail::check(bluebus_adapter_scan(handle_.get(), f, duration_ms, &infos, &n));
        });
        return DeviceInfo::take(infos, n);
    }
    /** Starts a callback scan; it runs until the returned object is destroyed. */
    Scan scan_start(const ScanFilter &filter, Scan::Callback callback) const {
        Scan s;
        s.callback_ = std::make_unique<Scan::Callback>(std::move(callback));
        bluebus_scan *raw = nullptr;
        filter.with_c([&](const bluebus_scan_filter *f) {
            detail::check(bluebus_adapter_scan_start(handle_.get(), f, &Scan::trampoline,
                                                     s.callback_.get(), &raw));
        });
        s.handle_.reset(raw);
        return s;
    }
    Device find_device(const ScanFilter &filter, uint32_t timeout_ms) const {
        bluebus_device *raw = nullptr;
        filter.with_c([&](const bluebus_scan_filter *f) {
            detail::check(bluebus_adapter_find_device(handle_.get(), f, timeout_ms, &raw));
        });
        return Device(raw);
    }
    Device device(const std::string &address) const {
        bluebus_device *raw = nullptr;
        detail::check(bluebus_adapter_device(handle_.get(), address.c_str(), &raw));
        return Device(raw);
    }
    std::vector<DeviceInfo> devices() const {
        bluebus_device_info *infos = nullptr;
        size_t n = 0;
        detail::check(bluebus_adapter_devices(handle_.get(), &infos, &n));
        return DeviceInfo::take(infos, n);
    }
    void remove_device(const std::string &address) const {
        detail::check(bluebus_adapter_remove_device(handle_.get(), address.c_str()));
    }
    PeripheralBuilder peripheral() const {
        bluebus_peripheral_builder *raw = nullptr;
        detail::check(bluebus_peripheral_builder_new(handle_.get(), &raw));
        return PeripheralBuilder(raw);
    }
    bluebus_adapter *get() const noexcept { return handle_.get(); }

private:
    detail::Handle<bluebus_adapter, bluebus_adapter_free> handle_;
};

/** The connection to BlueZ. */
class Session {
public:
    Session() {
        bluebus_session *raw = nullptr;
        detail::check(bluebus_session_new(&raw));
        handle_.reset(raw);
    }

    std::vector<AdapterInfo> adapters() const {
        bluebus_adapter_info *infos = nullptr;
        size_t n = 0;
        detail::check(bluebus_session_adapters(handle_.get(), &infos, &n));
        std::vector<AdapterInfo> out;
        try {
            for (size_t k = 0; k < n; ++k) {
                AdapterInfo a;
                a.name = detail::str(infos[k].name);
                a.path = detail::str(infos[k].path);
                a.address = detail::str(infos[k].address);
                a.alias = detail::str(infos[k].alias);
                a.powered = infos[k].powered;
                a.discovering = infos[k].discovering;
                out.push_back(std::move(a));
            }
        } catch (...) {
            bluebus_adapter_info_free(infos, n);
            throw;
        }
        bluebus_adapter_info_free(infos, n);
        return out;
    }
    /** Opens an adapter by name ("hci1"), path or address. */
    Adapter adapter(const std::string &name) const {
        bluebus_adapter *raw = nullptr;
        detail::check(bluebus_session_adapter(handle_.get(), name.c_str(), &raw));
        return Adapter(raw);
    }
    Adapter default_adapter() const {
        bluebus_adapter *raw = nullptr;
        detail::check(bluebus_session_default_adapter(handle_.get(), &raw));
        return Adapter(raw);
    }
    /** Agent accepting every confirmation ("Just Works"). */
    void set_agent_auto() const { detail::check(bluebus_session_set_agent_auto(handle_.get())); }
    /** Agent answering passkey requests with @p passkey. */
    void set_agent_passkey(uint32_t passkey) const {
        detail::check(bluebus_session_set_agent_passkey(handle_.get(), passkey));
    }
    void remove_agent() const { detail::check(bluebus_session_remove_agent(handle_.get())); }
    bluebus_session *get() const noexcept { return handle_.get(); }

private:
    detail::Handle<bluebus_session, bluebus_session_free> handle_;
};

/** Full 128-bit form of a UUID ("2a19" → "00002a19-0000-1000-8000-00805f9b34fb"). */
inline std::string normalize_uuid(const std::string &uuid) {
    char buf[37];
    detail::check(bluebus_normalize_uuid(uuid.c_str(), buf, sizeof buf));
    return buf;
}

/** Library version, e.g. "0.1.0". */
inline std::string version() { return bluebus_version(); }

}  // namespace bluebus

#endif /* BLUEBUS_HPP */
