# Contributing to bluebus

Bug reports, questions and pull requests are welcome.

## Reporting a bug

Open an [issue](https://github.com/bluebus-rs/bluebus/issues/new/choose)
with the smallest code that shows the problem, your bluebus and BlueZ
versions (`bluetoothctl --version`) and the adapter you use. Output of
`btmon` or `RUST_LOG=trace` helps for anything timing related.

## Pull requests

`main` is protected: every change goes through a pull request from a fork,
and the CI checks must pass before it is merged.

1. Fork the repository and create a branch.
2. Make the change, with a test for it. Most features can be tested
   against the mock BlueZ service in `tests/common` (no Bluetooth hardware
   needed, only `dbus-daemon`).
3. Run:

   ```sh
   cargo fmt --all
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```

4. For changes to what is sent to BlueZ, please also run the end-to-end
   test between two virtual controllers (see "Testing" in the README).
5. Add a line to `CHANGELOG.md` for user-visible changes, and open the
   pull request.

bluebus follows semantic versioning; `cargo semver-checks` should report no
breaking changes for a minor release.
