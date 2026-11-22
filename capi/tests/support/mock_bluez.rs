//! Runs a program against the mock BlueZ of the tests:
//!
//! ```sh
//! cargo run -p bluebus-c --example mock_bluez -- ./bluebus_test_c [args...]
//! ```
//!
//! Starts a private `dbus-daemon` with the mock `org.bluez`, runs the
//! program with `DBUS_SYSTEM_BUS_ADDRESS` pointing at it and exits with the
//! program's exit status. CTest uses it for the C and C++ tests.

#[path = "mod.rs"]
mod support;

fn main() {
    let mut args = std::env::args_os().skip(1);
    let Some(program) = args.next() else {
        eprintln!("usage: mock_bluez <program> [args...]");
        std::process::exit(2);
    };
    let mock = support::Mock::start();
    let status = std::process::Command::new(&program)
        .args(args)
        .env("DBUS_SYSTEM_BUS_ADDRESS", mock.address())
        .status();
    drop(mock);
    match status {
        Ok(s) => std::process::exit(s.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("cannot run {}: {e}", program.to_string_lossy());
            std::process::exit(2);
        }
    }
}
