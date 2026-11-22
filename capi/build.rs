// The shared library names itself (soname), so programs find it by name
// and rpath instead of by the path it was linked from.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libbluebus.so");
    }
}
