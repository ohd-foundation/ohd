# `ohd-storage-bindings`

Foreign-language bindings to the OHD Storage Rust core (`ohd-storage-core`).
One source crate, three downstream targets:

| Target | Tooling | Output | Importable as |
|---|---|---|---|
| **Android** (Kotlin) | uniffi 0.28 + cargo-ndk | `libohd_storage_bindings.so` per ABI + `ohd_storage.kt` | `package uniffi.ohd_storage` |
| **iOS** (Swift) | uniffi 0.28 + xcframework | static archive + `ohd_storage.swift` | `import OhdStorage` (TBD) |
| **Python** (CPython 3.11+) | PyO3 0.28 + maturin | `ohd_storage-*.whl` | `import ohd_storage` |

The same `ohd-storage-core` Rust API drives all three. uniffi covers
Android + iOS and PyO3 covers Python — both layers compile into the same
cdylib (`_uniffi_*` symbols vs `PyInit_*` symbols don't collide).

## Surface

The PyO3 module mirrors the uniffi facade one-for-one:

```python
import ohd_storage

# Open or create a per-user storage file.
s = ohd_storage.OhdStorage.create(path="/var/lib/ohd/data.db", key_hex="")
s = ohd_storage.OhdStorage.open(path="/var/lib/ohd/data.db", key_hex="")

# Identity.
s.user_ulid()                              # → Crockford-base32 ULID
token = s.issue_self_session_token()       # → "ohds_..." (cleartext, store securely)

# Events.
ulid = s.put_event(ohd_storage.EventInputDto(
    timestamp_ms=1_700_000_000_000,
    event_type="std.blood_glucose",
    channels=[ohd_storage.ChannelValueDto(
        channel_path="value",
        value_kind=ohd_storage.ValueKind.REAL,
        real_value=5.4,
    )],
))
events = s.query_events(ohd_storage.EventFilterDto(
    from_ms=0,
    to_ms=2_000_000_000_000,
    event_types_in=["std.blood_glucose"],
))

# Versions.
ohd_storage.format_version()    # "1.0"
ohd_storage.protocol_version()  # "ohdc.v0"
ohd_storage.storage_version()   # crate version

# Errors raise typed exceptions.
try:
    s.put_event(bad_input)
except ohd_storage.InvalidInput as e:
    ...
except ohd_storage.NotFound:
    ...
except ohd_storage.OhdError as e:   # root class — catches all
    ...
```

The five concrete exception classes (`OpenFailed`, `Auth`, `InvalidInput`,
`NotFound`, `Internal`) all subclass the root `OhdError` (which itself
subclasses `RuntimeError`), so a generic `except OhdError` catches every
storage-side failure.

## Python wheel — build & install

### Prerequisites

| Tool | Version | Why |
|---|---|---|
| Rust toolchain | 1.88+ (workspace MSRV) | Per `storage/rust-toolchain.toml`. |
| Python | 3.11+ | `abi3-py311` wheel covers 3.11 → 3.14+. |
| `maturin` | 1.7+ | `pip install maturin` or `pipx install maturin`. |

The wheel is **abi3-py311**: one wheel works on every CPython 3.11+
ABI without a per-minor rebuild. We don't ship per-version wheels.

### Build

From this crate:

```bash
cd storage/crates/ohd-storage-bindings
maturin build --release
# → wheel lands in target/wheels/ohd_storage-<ver>-cp311-abi3-<plat>.whl
```

The cargo features list in `pyproject.toml` (`features = ["pyo3",
"extension-module"]`) is applied automatically — no `--features` flag
needed.

If you don't have `maturin` on your PATH and don't want to install it
permanently, `python3 -m maturin build --release` works the same way
once you've `pip install maturin`-ed it.

### Install

```bash
pip install target/wheels/ohd_storage-*.whl
python -c "import ohd_storage; print(ohd_storage.format_version())"
# → 1.0
```

### Develop loop

```bash
maturin develop --release          # builds + installs into the active venv
pytest tests/                       # run the smoke tests
```

`maturin develop` skips the wheel packaging step (faster) and installs
the `.so` directly into your active Python's `site-packages`. Re-run on
every Rust source change.

### Tests

A minimal `pytest` smoke suite lives under
`crates/ohd-storage-bindings/tests/test_pyo3.py`. It exercises the open
→ put → query round-trip on a tempfile-backed storage file and verifies
the typed exceptions for bad input. Run with:

```bash
pip install -e ".[dev]"     # installs pytest in addition to the wheel
pytest tests/
```

The Rust-side `cargo test -p ohd-storage-bindings --features pyo3`
compiles the PyO3 surface (no Python interpreter required) and runs the
uniffi smoke checks.

## Android `.aar` / `.so` (uniffi)

See `connect/android/BUILD.md` for the full recipe. Two-stage:

1. **Stage 1** — cross-compile per ABI:
   ```bash
   cd storage/crates/ohd-storage-bindings
   cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 \
     -o ../../../connect/android/app/src/main/jniLibs \
     build --release
   ```
2. **Stage 2** — generate Kotlin façade:
   ```bash
   cargo run --features cli --bin uniffi-bindgen -- \
     generate \
     --library target/release/libohd_storage_bindings.so \
     --language kotlin \
     --out-dir ../../../connect/android/app/src/main/java/uniffi
   ```

The Kotlin sealed-class exception hierarchy (`OhdException.OpenFailed`,
`OhdException.Auth`, …) and the data classes (`EventInputDto`,
`PutEventOutcomeDto`, …) are generated by uniffi-bindgen.

## iOS `.xcframework` (uniffi) — TBD

`connect/ios/BUILD.md` will land alongside the iOS app. The recipe is a
straight extension of Android:

```bash
cargo build --target aarch64-apple-ios --release
cargo build --target aarch64-apple-ios-sim --release
xcodebuild -create-xcframework \
  -library target/aarch64-apple-ios/release/libohd_storage_bindings.a \
  -library target/aarch64-apple-ios-sim/release/libohd_storage_bindings.a \
  -output OhdStorage.xcframework
cargo run --features cli --bin uniffi-bindgen -- generate \
  --library target/aarch64-apple-ios/release/libohd_storage_bindings.dylib \
  --language swift \
  --out-dir OhdStorage.swift/
```

`crate-type = ["cdylib", "staticlib", "rlib"]` already declares the
right artefact (`staticlib` for the iOS xcframework).

## Why two binding layers

uniffi 0.28 *does* speak Python. We still ship a separate PyO3 module
because:

1. **Server-side scripting** is mentioned in
   `spec/components/storage.md` — the conformance harness drives storage
   from Python. PyO3 is the idiomatic Python-from-Rust path; uniffi's
   Python codegen is a second-class consumer (uniffi was designed
   Kotlin-first by Mozilla).
2. **Wheel packaging.** maturin is the standard tool for Rust→Python
   wheels; uniffi's Python codegen ships loose `.py` files that consumers
   would have to wrap into a wheel themselves.
3. **GIL handling.** PyO3 lets us release the GIL via `Python::detach`
   around long SQLite work; uniffi's Python codegen doesn't.
4. **Typed exceptions.** PyO3's `create_exception!` produces real
   subclassable Python exception classes; uniffi maps to a dataclass
   union the user has to inspect.

Keeping both pays off: Android / iOS go through uniffi (which is what
those platforms expect), Python goes through PyO3 (which is what Python
consumers expect). One Rust source surface, two ergonomic foreign-language
APIs.

## Layout

```
crates/ohd-storage-bindings/
├── Cargo.toml                ← features: cli, pyo3, extension-module
├── pyproject.toml            ← maturin config
├── README.md                 ← this file
├── src/
│   ├── lib.rs                ← uniffi facade (Kotlin / Swift / uniffi-Python)
│   ├── pyo3_module.rs        ← PyO3 facade (Python wheel)
│   └── uniffi-bindgen.rs     ← uniffi-bindgen CLI binary (under --features cli)
└── tests/
    └── test_pyo3.py          ← Python smoke tests (pytest)
```

`target/wheels/`, `target/release/libohd_storage_bindings.so`, and the
generated Kotlin / Swift sources are gitignored.

## License

Dual-licensed `Apache-2.0 OR MIT` — see [`../../../spec/LICENSE`](../../../spec/LICENSE).
