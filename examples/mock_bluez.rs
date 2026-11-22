//! The mock `org.bluez` service of the test suite as a standalone process,
//! for tests written in other languages (the Python bindings' pytest suite).
//!
//! Starts a private `dbus-daemon` with the mock BlueZ (hci0 finds
//! "Headset" 11:22:33:44:55:66 and an anonymous AA:BB:CC:DD:EE:FF; hci1
//! knows "Other"), prints `ADDRESS=<bus address>` and then answers commands
//! on stdin, one per line, each with one line on stdout:
//!
//! | command                     | effect / answer                                     |
//! |-----------------------------|-----------------------------------------------------|
//! | `notify <chr path> <hex>`   | the remote characteristic sends a notification      |
//! | `rssi <device path> <n>`    | a new advertisement changes the RSSI                |
//! | `write_local <uuid> <hex>`  | a remote central writes a local characteristic      |
//! | `read_local <uuid>`         | a remote central reads it: `ok <hex>`               |
//! | `state`                     | `ok <json>`: agents, GATT apps, advertisements, …   |
//! | `quit` (or EOF / SIGTERM)   | stops the daemon and exits                          |
//!
//! Answers are `ok …` or `error <message>`.
//!
//! ```sh
//! cargo run --example mock_bluez
//! ```

#[path = "../tests/common/mod.rs"]
mod common;

use std::collections::HashMap;
use std::io::Write;

use tokio::io::{AsyncBufReadExt, BufReader};
use zbus::zvariant::{ObjectPath, OwnedValue, Str};

use common::*;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) {
        return Err(format!("odd hex length: {s}"));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| format!("bad hex {s}: {e}")))
        .collect()
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_list(items: impl IntoIterator<Item = String>) -> String {
    format!("[{}]", items.into_iter().collect::<Vec<_>>().join(","))
}

/// A D-Bus value as JSON (strings, string lists, numbers, booleans; other
/// values as their textual form).
fn json_value(v: &OwnedValue) -> String {
    if let Ok(s) = v.downcast_ref::<Str>() {
        return json_str(s.as_str());
    }
    if let Ok(b) = v.downcast_ref::<bool>() {
        return b.to_string();
    }
    if let Ok(Ok(list)) = v.try_clone().map(Vec::<String>::try_from) {
        return json_list(list.iter().map(|s| json_str(s)));
    }
    let number = v
        .downcast_ref::<u16>()
        .map(|n| n.to_string())
        .or_else(|_| v.downcast_ref::<i16>().map(|n| n.to_string()))
        .or_else(|_| v.downcast_ref::<u32>().map(|n| n.to_string()))
        .or_else(|_| v.downcast_ref::<u8>().map(|n| n.to_string()));
    number.unwrap_or_else(|_| json_str(&format!("{v:?}")))
}

fn json_props(props: &HashMap<String, OwnedValue>) -> String {
    let mut keys: Vec<&String> = props.keys().collect();
    keys.sort();
    format!(
        "{{{}}}",
        keys.iter()
            .map(|k| format!("{}:{}", json_str(k), json_value(&props[*k])))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn state_json(bz: &MockBluez) -> String {
    let st = bz.state.lock().unwrap();
    let agents = json_list(st.agents.iter().map(|(owner, path, cap)| {
        format!(
            "{{\"owner\":{},\"path\":{},\"capability\":{}}}",
            json_str(owner.as_str()),
            json_str(path.as_str()),
            json_str(cap)
        )
    }));
    let mut apps: Vec<_> = st.gatt_apps.iter().collect();
    apps.sort_by(|a, b| a.0.cmp(b.0));
    let apps = json_list(apps.into_iter().map(|((owner, path), objects)| {
        let mut objs: Vec<_> = objects.iter().collect();
        objs.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        let objs = json_list(objs.into_iter().map(|(p, ifaces)| {
            let mut names: Vec<&String> = ifaces.keys().collect();
            names.sort();
            format!(
                "{{\"path\":{},\"interfaces\":{{{}}}}}",
                json_str(p.as_str()),
                names
                    .iter()
                    .map(|n| format!("{}:{}", json_str(n), json_props(&ifaces[*n])))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }));
        format!(
            "{{\"owner\":{},\"path\":{},\"objects\":{objs}}}",
            json_str(owner),
            json_str(path)
        )
    }));
    let mut advs: Vec<_> = st.advertisements.iter().collect();
    advs.sort_by(|a, b| a.0.cmp(b.0));
    let advs = json_list(advs.into_iter().map(|((owner, path), props)| {
        format!(
            "{{\"owner\":{},\"path\":{},\"properties\":{}}}",
            json_str(owner),
            json_str(path),
            json_props(props)
        )
    }));
    let filter = match &st.discovery_filter {
        Some(f) => json_props(f),
        None => "null".into(),
    };
    let writes = json_list(st.remote_writes.iter().map(|(path, value, kind)| {
        format!(
            "{{\"path\":{},\"value\":{},\"type\":{}}}",
            json_str(path),
            json_str(&hex(value)),
            kind.as_deref()
                .map(json_str)
                .unwrap_or_else(|| "null".into())
        )
    }));
    format!(
        "{{\"agents\":{agents},\"default_agent\":{},\"gatt_apps\":{apps},\
         \"advertisements\":{advs},\"discovery_filter\":{filter},\
         \"pair_requests\":{},\"remote_writes\":{writes}}}",
        st.default_agent.is_some(),
        json_list(st.pair_requests.iter().map(|s| json_str(s)))
    )
}

/// (owner, object path) of the registered local characteristic `uuid`.
fn local_characteristic(bz: &MockBluez, uuid: &str) -> Result<(String, String), String> {
    let uuid = bluebus::ble::normalize_uuid(uuid).map_err(|e| e.to_string())?;
    let st = bz.state.lock().unwrap();
    for ((owner, _), objects) in &st.gatt_apps {
        for (path, ifaces) in objects {
            let Some(chr) = ifaces.get("org.bluez.GattCharacteristic1") else {
                continue;
            };
            let found = chr
                .get("UUID")
                .and_then(|v| v.downcast_ref::<Str>().ok())
                .is_some_and(|u| u.as_str().eq_ignore_ascii_case(&uuid));
            if found {
                return Ok((owner.clone(), path.to_string()));
            }
        }
    }
    Err(format!("no registered local characteristic {uuid}"))
}

async fn command(bz: &MockBluez, line: &str) -> Result<String, String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.as_slice() {
        ["notify", path, value] => {
            remote_notify(&bz.server, path, unhex(value)?).await;
            Ok(String::new())
        }
        ["rssi", path, rssi] => {
            let rssi: i16 = rssi.parse().map_err(|e| format!("bad rssi: {e}"))?;
            set_device_rssi(&bz.server, path, rssi).await;
            Ok(String::new())
        }
        ["write_local", uuid, value] => {
            let (owner, path) = local_characteristic(bz, uuid)?;
            let path = ObjectPath::try_from(path.as_str()).map_err(|e| e.to_string())?;
            let options: HashMap<String, OwnedValue> = HashMap::new();
            bz.server
                .call_method(
                    Some(owner.as_str()),
                    path,
                    Some("org.bluez.GattCharacteristic1"),
                    "WriteValue",
                    &(unhex(value)?, options),
                )
                .await
                .map_err(|e| e.to_string())?;
            Ok(String::new())
        }
        ["read_local", uuid] => {
            let (owner, path) = local_characteristic(bz, uuid)?;
            let path = ObjectPath::try_from(path.as_str()).map_err(|e| e.to_string())?;
            let options: HashMap<String, OwnedValue> = HashMap::new();
            let reply = bz
                .server
                .call_method(
                    Some(owner.as_str()),
                    path,
                    Some("org.bluez.GattCharacteristic1"),
                    "ReadValue",
                    &(options,),
                )
                .await
                .map_err(|e| e.to_string())?;
            let value: Vec<u8> = reply.body().deserialize().map_err(|e| e.to_string())?;
            Ok(hex(&value))
        }
        ["state"] => Ok(state_json(bz)),
        _ => Err(format!("unknown command: {line}")),
    }
}

#[tokio::main]
async fn main() {
    let bz = MockBluez::start().await;
    println!("ADDRESS={}", bz.bus.address);
    std::io::stdout().flush().unwrap();

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("SIGTERM handler");
    loop {
        let line = tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(line)) => line,
                _ => break,
            },
            _ = term.recv() => break,
            _ = tokio::signal::ctrl_c() => break,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "quit" {
            println!("ok");
            break;
        }
        match command(&bz, line).await {
            Ok(answer) if answer.is_empty() => println!("ok"),
            Ok(answer) => println!("ok {answer}"),
            Err(e) => println!("error {}", e.replace('\n', " ")),
        }
        std::io::stdout().flush().unwrap();
    }
    // Dropping `bz` stops the private dbus-daemon.
    drop(bz);
}
