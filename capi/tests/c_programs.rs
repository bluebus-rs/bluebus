//! Compiles the header, the C and C++ tests and the examples with the
//! system compilers (`cc`/`c++`, override with `CC`/`CXX`), checks the
//! header against the library (exported symbols, struct layouts) and runs
//! the test programs against the mock BlueZ.

#[path = "../src/lib.rs"]
#[allow(dead_code, unused_imports)]
mod ffi;
mod support;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use support::Mock;

fn capi_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn include_dir() -> PathBuf {
    capi_dir().join("include")
}

fn out_dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("bluebus-c-tests");
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn tool(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.to_owned())
}

/// The shared and static library, built by cargo (once per test binary).
struct Lib {
    shared: PathBuf,
    staticlib: PathBuf,
}

fn lib() -> &'static Lib {
    static LIB: OnceLock<Lib> = OnceLock::new();
    LIB.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let out = Command::new(cargo)
            .args(["build", "-p", "bluebus-c", "--message-format=json"])
            .current_dir(capi_dir())
            .output()
            .expect("run cargo");
        assert!(
            out.status.success(),
            "cargo build -p bluebus-c failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        let mut files = Vec::new();
        for line in stdout.lines() {
            if !line.contains("\"compiler-artifact\"") || !line.contains("bluebus-c") {
                continue;
            }
            let Some(start) = line.find("\"filenames\":[") else {
                continue;
            };
            let rest = &line[start + 13..];
            let list = &rest[..rest.find(']').unwrap()];
            files.extend(list.split(',').map(|s| PathBuf::from(s.trim_matches('"'))));
        }
        let find = |name: &str| {
            files
                .iter()
                .find(|f| f.file_name().is_some_and(|n| n == name))
                .unwrap_or_else(|| panic!("{name} not among {files:?}"))
                .clone()
        };
        Lib {
            shared: find("libbluebus.so"),
            staticlib: find("libbluebus.a"),
        }
    })
}

#[track_caller]
fn run(cmd: &mut Command) -> String {
    let out = cmd.output().unwrap_or_else(|e| panic!("{cmd:?}: {e}"));
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{cmd:?} failed:\n{text}");
    text
}

fn compile_c(source: &Path, exe: &Path, std: &str, link: bool, statically: bool) {
    let mut cmd = Command::new(tool("CC", "cc"));
    cmd.arg(format!("-std={std}"))
        .args(["-Wall", "-Wextra", "-Wpedantic", "-Werror", "-I"])
        .arg(include_dir())
        .arg(source)
        .arg("-o")
        .arg(exe);
    if !link {
        cmd.arg("-c");
    }
    link_args(&mut cmd, link, statically);
    run(&mut cmd);
}

fn compile_cpp(source: &Path, exe: &Path, link: bool) {
    let mut cmd = Command::new(tool("CXX", "c++"));
    cmd.args([
        "-std=c++17",
        "-Wall",
        "-Wextra",
        "-Wpedantic",
        "-Werror",
        "-I",
    ])
    .arg(include_dir())
    .arg(source)
    .arg("-o")
    .arg(exe);
    if !link {
        cmd.arg("-c");
    }
    link_args(&mut cmd, link, false);
    run(&mut cmd);
}

fn link_args(cmd: &mut Command, link: bool, statically: bool) {
    if !link {
        return;
    }
    let lib = lib();
    if statically {
        cmd.arg(&lib.staticlib)
            .args(["-lpthread", "-ldl", "-lm", "-lrt"]);
    } else {
        let dir = lib.shared.parent().unwrap();
        cmd.arg("-L")
            .arg(dir)
            .arg("-lbluebus")
            .arg(format!("-Wl,-rpath,{}", dir.display()));
    }
}

/// Runs a test program with the mock BlueZ as its system bus.
fn run_against_mock(exe: &Path) -> String {
    let mock = Mock::start();
    let out = run(Command::new(exe).env("DBUS_SYSTEM_BUS_ADDRESS", mock.address()));
    drop(mock);
    out
}

// ---------------------------------------------------------------------------

#[test]
fn header_compiles_as_c_and_cpp() {
    let dir = out_dir();
    let c = dir.join("header.c");
    std::fs::write(&c, "#include \"bluebus.h\"\nint main(void) { return 0; }\n").unwrap();
    for std in ["c99", "c11", "c17"] {
        compile_c(&c, &dir.join(format!("header-{std}.o")), std, false, false);
    }
    let cpp = dir.join("header.cpp");
    std::fs::write(
        &cpp,
        "#include \"bluebus.h\"\n#include \"bluebus.hpp\"\nint main() { return 0; }\n",
    )
    .unwrap();
    compile_cpp(&cpp, &dir.join("header-cpp.o"), false);
    // bluebus.h alone in C++11.
    let cpp11 = dir.join("header11.cpp");
    std::fs::write(&cpp11, "#include \"bluebus.h\"\nint main() { return 0; }\n").unwrap();
    run(Command::new(tool("CXX", "c++"))
        .args([
            "-std=c++11",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
            "-c",
            "-I",
        ])
        .arg(include_dir())
        .arg(&cpp11)
        .arg("-o")
        .arg(dir.join("header11.o")));
}

#[test]
fn header_version_matches_the_crate() {
    let h = std::fs::read_to_string(include_dir().join("bluebus.h")).unwrap();
    let version = env!("CARGO_PKG_VERSION");
    assert!(h.contains(&format!("#define BLUEBUS_VERSION_STRING \"{version}\"")));
    let mut parts = version.split('.');
    for name in ["MAJOR", "MINOR", "PATCH"] {
        let v = parts.next().unwrap();
        assert!(
            h.contains(&format!("#define BLUEBUS_VERSION_{name} {v}\n")),
            "BLUEBUS_VERSION_{name}"
        );
    }
}

/// Function names declared in bluebus.h.
fn declared_functions() -> BTreeSet<String> {
    let h = std::fs::read_to_string(include_dir().join("bluebus.h")).unwrap();
    let mut names = BTreeSet::new();
    let mut rest = h.as_str();
    while let Some(i) = rest.find("bluebus_") {
        let tail = &rest[i..];
        let end = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(tail.len());
        let name = &tail[..end];
        let before = rest[..i].chars().last();
        // A declaration: `type name(` at the start of a line or after `*`/space.
        if tail[end..].starts_with('(')
            && before != Some('(')
            && !before.is_some_and(|c| c.is_alphanumeric())
        {
            let line_start = rest[..i].rfind('\n').map_or(0, |n| n + 1);
            let line = &rest[line_start..i];
            if !line.trim_start().starts_with('*')
                && !line.contains("typedef")
                && !line.trim_start().starts_with("/")
            {
                names.insert(name.to_owned());
            }
        }
        rest = &tail[end..];
    }
    names
}

#[test]
fn header_declares_exactly_the_exported_functions() {
    let declared = declared_functions();
    let nm = run(Command::new("nm")
        .args(["-D", "--defined-only"])
        .arg(&lib().shared));
    let exported: BTreeSet<String> = nm
        .lines()
        .filter_map(|l| l.split_whitespace().nth(2))
        .filter(|s| s.starts_with("bluebus_"))
        .map(str::to_owned)
        .collect();
    assert!(declared.len() > 60, "parsed {declared:?}");
    let missing: Vec<_> = declared.difference(&exported).collect();
    let undeclared: Vec<_> = exported.difference(&declared).collect();
    assert!(
        missing.is_empty() && undeclared.is_empty(),
        "declared but not exported: {missing:?}\nexported but not declared: {undeclared:?}"
    );
}

#[test]
fn struct_layouts_match() {
    use std::mem::{align_of, offset_of, size_of};
    let mut expected = String::new();
    let mut c = String::from(
        "#include <stdio.h>\n#include <stddef.h>\n#include \"bluebus.h\"\nint main(void) {\n",
    );
    macro_rules! layout {
        ($c_name:literal, $ty:ty, [$($field:ident),*]) => {{
            writeln!(expected, "{} size {} align {}", $c_name, size_of::<$ty>(), align_of::<$ty>()).unwrap();
            writeln!(c, "  printf(\"{} size %zu align %zu\\n\", sizeof({}), _Alignof({}));", $c_name, $c_name, $c_name).unwrap();
            $(
                writeln!(expected, "{}.{} {}", $c_name, stringify!($field), offset_of!($ty, $field)).unwrap();
                writeln!(c, "  printf(\"{}.{} %zu\\n\", offsetof({}, {}));", $c_name, stringify!($field), $c_name, stringify!($field)).unwrap();
            )*
        }};
    }
    layout!(
        "bluebus_manufacturer_data",
        ffi::bluebus_manufacturer_data,
        [company_id, data, len]
    );
    layout!(
        "bluebus_service_data",
        ffi::bluebus_service_data,
        [uuid, data, len]
    );
    layout!(
        "bluebus_device_info",
        ffi::bluebus_device_info,
        [
            address,
            address_type,
            name,
            alias,
            rssi,
            tx_power,
            has_rssi,
            has_tx_power,
            connected,
            paired,
            trusted,
            services_resolved,
            uuids,
            uuid_count,
            manufacturer_data,
            manufacturer_data_count,
            service_data,
            service_data_count
        ]
    );
    layout!(
        "bluebus_adapter_info",
        ffi::bluebus_adapter_info,
        [name, path, address, alias, powered, discovering]
    );
    layout!(
        "bluebus_gatt_attribute",
        ffi::bluebus_gatt_attribute,
        [kind, uuid, path, service_uuid, characteristic_uuid, flags]
    );
    layout!(
        "bluebus_scan_filter",
        ffi::bluebus_scan_filter,
        [
            transport,
            name,
            name_prefix,
            address,
            service_uuids,
            service_uuid_count,
            min_rssi,
            has_min_rssi,
            duplicate_data,
            has_duplicate_data
        ]
    );
    writeln!(
        expected,
        "bluebus_status size {}",
        size_of::<ffi::bluebus_status>()
    )
    .unwrap();
    writeln!(
        c,
        "  printf(\"bluebus_status size %zu\\n\", sizeof(bluebus_status));"
    )
    .unwrap();
    c.push_str("  return 0;\n}\n");

    let dir = out_dir();
    let src = dir.join("layout.c");
    std::fs::write(&src, c).unwrap();
    let exe = dir.join("layout");
    compile_c(&src, &exe, "c11", true, false);
    let got = run(&mut Command::new(&exe));
    assert_eq!(got, expected);
}

#[test]
fn constants_match() {
    let h = std::fs::read_to_string(include_dir().join("bluebus.h")).unwrap();
    for (bit, name) in ffi::FLAGS {
        let macro_name = format!("BLUEBUS_FLAG_{}", name.to_uppercase().replace('-', "_"));
        let shift = bit.trailing_zeros();
        let decl = h
            .lines()
            .find(|l| l.starts_with(&format!("#define {macro_name} ")))
            .unwrap_or_else(|| panic!("{macro_name} missing"));
        assert!(decl.ends_with(&format!("(1u << {shift})")), "{decl}");
    }
    use ffi::bluebus_status::*;
    for (status, name) in [
        (BLUEBUS_OK, "BLUEBUS_OK"),
        (BLUEBUS_ERR_TIMEOUT, "BLUEBUS_ERR_TIMEOUT"),
        (BLUEBUS_ERR_NOT_FOUND, "BLUEBUS_ERR_NOT_FOUND"),
        (BLUEBUS_ERR_INVALID_ARGUMENT, "BLUEBUS_ERR_INVALID_ARGUMENT"),
        (BLUEBUS_ERR_DBUS, "BLUEBUS_ERR_DBUS"),
        (BLUEBUS_ERR_CLOSED, "BLUEBUS_ERR_CLOSED"),
        (BLUEBUS_ERR_INTERNAL, "BLUEBUS_ERR_INTERNAL"),
    ] {
        assert!(
            h.contains(&format!("{name} = {},", status as i32))
                || h.contains(&format!("{name} = {} ", status as i32)),
            "{name}"
        );
    }
    for (value, name) in [
        (ffi::TRANSPORT_DEFAULT, "BLUEBUS_TRANSPORT_DEFAULT"),
        (ffi::TRANSPORT_AUTO, "BLUEBUS_TRANSPORT_AUTO"),
        (ffi::TRANSPORT_BREDR, "BLUEBUS_TRANSPORT_BREDR"),
        (ffi::TRANSPORT_LE, "BLUEBUS_TRANSPORT_LE"),
        (ffi::SCAN_FOUND, "BLUEBUS_SCAN_FOUND"),
        (ffi::SCAN_UPDATED, "BLUEBUS_SCAN_UPDATED"),
        (ffi::ATTR_SERVICE, "BLUEBUS_ATTR_SERVICE"),
        (ffi::ATTR_CHARACTERISTIC, "BLUEBUS_ATTR_CHARACTERISTIC"),
        (ffi::ATTR_DESCRIPTOR, "BLUEBUS_ATTR_DESCRIPTOR"),
    ] {
        assert!(h.contains(&format!("{name} = {value}")), "{name}");
    }
}

#[test]
fn c_test_program_shared() {
    let exe = out_dir().join("test_c_shared");
    compile_c(&capi_dir().join("tests/test.c"), &exe, "c99", true, false);
    let out = run_against_mock(&exe);
    assert!(out.contains("C checks passed"), "{out}");
}

#[test]
fn c_test_program_static() {
    let exe = out_dir().join("test_c_static");
    compile_c(&capi_dir().join("tests/test.c"), &exe, "c99", true, true);
    let out = run_against_mock(&exe);
    assert!(out.contains("C checks passed"), "{out}");
}

#[test]
fn cpp_test_program() {
    let exe = out_dir().join("test_cpp");
    compile_cpp(&capi_dir().join("tests/test.cpp"), &exe, true);
    let out = run_against_mock(&exe);
    assert!(out.contains("C++ checks passed"), "{out}");
}

#[test]
fn examples_compile() {
    let dir = out_dir();
    for c in ["scan.c", "peripheral.c"] {
        compile_c(
            &capi_dir().join("examples").join(c),
            &dir.join(c.trim_end_matches(".c")),
            "c99",
            true,
            false,
        );
    }
    compile_cpp(
        &capi_dir().join("examples/central.cpp"),
        &dir.join("central"),
        true,
    );
}

#[test]
fn scan_example_runs_against_the_mock() {
    let exe = out_dir().join("scan_example");
    compile_c(
        &capi_dir().join("examples/scan.c"),
        &exe,
        "c99",
        true,
        false,
    );
    let mock = Mock::start();
    let out = run(Command::new(&exe)
        .args(["hci0", "300"])
        .env("DBUS_SYSTEM_BUS_ADDRESS", mock.address()));
    assert!(out.contains("11:22:33:44:55:66"), "{out}");
    assert!(out.contains("Headset"), "{out}");
}
