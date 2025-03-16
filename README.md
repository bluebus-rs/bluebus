# BlueBus

<a href="https://crates.io/crates/bluebus">
    <img style="display: inline!important" src="https://img.shields.io/crates/v/bluebus.svg"></img>
</a>
<a href="https://docs.rs/bluebus">
    <img style="display: inline!important" src="https://docs.rs/bluebus/badge.svg"></img>
</a>
<a href="https://docs.rs/bluebus">
    <img style="display: inline!important" src="https://img.shields.io/crates/d/bluebus"></img>
</a>

BlueBus is a Rust library for interacting with Bluetooth devices using the **zbus**. It provides an **async-first** interface for scanning, managing, and communicating with Bluetooth devices on Linux systems.

## Features
✅ **Device Discovery**: Scan for available Bluetooth devices.  
✅ **Device Management**: Connect, pair, unpair, and trust devices.  
✅ **GATT Support**: Read and write Bluetooth characteristics.  
✅ **Low Energy Advertisement**: Manage Bluetooth LE advertisements.  
✅ **Async Support**: Fully compatible with `tokio` and `async/await`.

## Installation
Add `bluebus` to your `Cargo.toml`:

```toml
[dependencies]
bluebus = "0.1.3"
```

## Example Usage
This example scans for Bluetooth devices and prints their addresses:

```rust
#[tokio::main]
async fn main() -> zbus::Result<()> {
    let conn = bluebus::get_system_connection().await.unwrap();
    let adapter = bluebus::AdapterProxy::builder(&conn)
        .path(bluebus::ADAPTER_PATH)?.build().await?;

    if !adapter.powered().await? {
        adapter.set_powered(true).await?;
    }

    adapter.start_discovery().await?;
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
    adapter.stop_discovery().await?;
    let monitor = Arc::new(bluebus::Monitor::new(conn.clone(), manager).await);
    let monitor_clone = Arc::clone(&monitor);
    bluebus::print_avaiable_devices(&objects);
    tokio::spawn(async move {monitor.monitor_device_added().await;});
    tokio::spawn(async move {monitor_clone.monitor_device_removed().await;});
    Ok(())
}
```

## Documentation
Detailed API documentation is available on [docs.rs](https://docs.rs/bluebus).

## License
BlueBus is licensed under either:
- [MIT License](LICENSE-MIT)
- [Apache License (Version 2.0)](LICENSE-APACHE)

## Contributing
Contributions are welcome! Feel free to open an issue or submit a pull request on [GitHub](https://github.com/bluebus-rs/bluebus).

---

## Acknowledgments

#### Special thanks to [REM Vision Lab](https://remvisionlab.com/) and [Sezai Acer](https://github.com/sezaiacers) for their support in improving this library.
