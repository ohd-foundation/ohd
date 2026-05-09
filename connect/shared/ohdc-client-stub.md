# OHDC client codegen — drop layout

> Placeholder: describes how the codegen'd OHDC client libraries land here per
> language, consumed by each Connect form factor. None of this exists yet —
> it's a layout contract for the implementation phase.

## Source of truth

The OHDC `.proto` schemas live in **OHD Storage** at:

```
../../storage/proto/ohdc/v0/*.proto
```

Storage owns:

- The `.proto` files (`ohdc/v0/auth.proto`, `events.proto`, `grants.proto`,
  `pending.proto`, `audit.proto`, `cases.proto`, `export.proto`,
  `notify.proto`, `diag.proto`, …).
- The `buf.yaml` / `buf.gen.yaml` configuration that drives codegen.
- The CI pipeline that runs `buf generate` and publishes per-language client
  packages.
- The Buf Schema Registry entry at `buf.build/openhealth-data/ohdc`.

Connect **does not** own any of the above. Connect consumes the generated
output.

## Drop layout

Per-language generated code lands in `shared/ohdc-clients/<lang>/`:

```
shared/ohdc-clients/
├── kotlin/                   # Maven coords: org.ohd:ohdc-client-kotlin:<version>
│   ├── build.gradle.kts
│   └── src/main/kotlin/org/ohd/ohdc/v0/...
│
├── swift/                    # SwiftPM: github.com/ohd-foundation/ohdc-client-swift
│   ├── Package.swift
│   └── Sources/OhdcClient/...
│
├── typescript/               # npm: @ohd/ohdc-client
│   ├── package.json
│   └── src/ohdc/v0/...
│
└── rust/                     # crates.io: ohdc-client (Cargo workspace member)
    ├── Cargo.toml
    └── src/ohdc/v0/...
```

Each drop ships:

1. The generated message types (Protobuf descriptors → idiomatic per-language
   structs / classes).
2. The Connect-RPC client stub for `OhdcService`.
3. Optional gRPC client stub (Connect-RPC is wire-compatible).
4. Streaming helpers for `ReadSamples`, `Export`, `Audit.Tail`.
5. `application/json` + `application/proto` encoding selection.
6. Error decoding for the OHDC structured-error model (`OUT_OF_SCOPE`,
   `INVALID_UNIT`, etc.).

## How each form factor consumes it

| Form factor | Consumption pattern |
|---|---|
| **Android** (`../android/`) | `app/build.gradle.kts` adds `implementation(project(":shared:ohdc-clients:kotlin"))` if the Kotlin drop is committed in-tree, or pulls from Maven once published. |
| **iOS** (`../ios/`) | `Package.swift` declares `.package(path: "../shared/ohdc-clients/swift")` (or a SwiftPM remote URL once published). |
| **Web** (`../web/`) | `package.json` declares `"@ohd/ohdc-client": "file:../shared/ohdc-clients/typescript"` (or the npm registry version). |
| **CLI** (`../cli/`) | `Cargo.toml` declares `ohdc-client = { path = "../shared/ohdc-clients/rust" }` (or crates.io once published). |
| **MCP** (`../mcp/`) | Same as web — `"@ohd/ohdc-client": "file:../shared/ohdc-clients/typescript"`. |

## Bootstrap

Once the storage component publishes the first codegen drop, Connect's
implementation phase:

1. Symlinks or copies `shared/ohdc-clients/<lang>/` from the storage build
   output (decision: in-tree commit vs. registry pull TBD per the storage
   release plan).
2. Each form factor adds the dependency line above.
3. The hello-world stubs become "fetch `Auth.WhoAmI`, render the result"
   smoke tests, validating the end-to-end client surface.

Until then this directory stays empty (other than this stub).

## Versioning

OHDC is versioned via the `.proto` package name (`ohdc.v0`, `ohdc.v2`).
Connect targets a single major version at a time; minor / additive changes
flow through automatically. Breaking changes require a new `shared/ohdc-
clients/<lang>/v2/` drop side-by-side with v1; both can coexist during the
migration window.

The Connect-side version pin lives in [`../STATUS.md`](../STATUS.md).
