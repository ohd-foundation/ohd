# OHD — native Linux packaging

Distribution-native packages for the five OHD binaries. This is the bare-metal / VM / home-server companion to the Docker recipes in each component's `deploy/`. See [`README.md`](README.md) for the project overview and [`DEPLOYMENT.md`](DEPLOYMENT.md) for the deployment matrix.

## Binaries

| Binary | Source | Type | Daemon? |
|---|---|---|---|
| `ohd-storage-server` | `storage/crates/ohd-storage-server/` | Rust binary | yes (systemd) |
| `ohd-relay` | `relay/` | Rust binary | yes (systemd) |
| `ohd-connect` | `connect/cli/` | Rust binary | no |
| `ohd-emergency` | `emergency/cli/` | Rust binary | no |
| `ohd-care` | `care/cli/` | Python wheel | no |

The four Rust binaries get .deb + .rpm + Arch PKGBUILDs from the
upstream Cargo manifests via `cargo-deb`, `cargo-generate-rpm`, and a
hand-written Arch PKGBUILD per binary. `ohd-care` ships as a Python
wheel; native packages wrap the wheel — see [`care/cli/PACKAGING.md`](care/cli/PACKAGING.md).

Per-component pointers:

- [`storage/README.md`](storage/README.md), [`storage/deploy/README.md`](storage/deploy/README.md)
- [`relay/README.md`](relay/README.md)
- [`connect/cli/README.md`](connect/cli/README.md)
- [`emergency/cli/README.md`](emergency/cli/README.md)
- [`care/cli/README.md`](care/cli/README.md), [`care/cli/PACKAGING.md`](care/cli/PACKAGING.md)

## Repository layout

```
ohd/
├── packaging/                       # ← cross-binary packaging tree (this PR)
│   ├── systemd/
│   │   ├── ohd-storage.service      # systemd unit for the storage daemon
│   │   └── ohd-relay.service        # systemd unit for the relay daemon
│   ├── debian/
│   │   ├── ohd-storage/             # postinst / prerm / postrm
│   │   ├── ohd-relay/
│   │   └── ohd-care/
│   ├── arch/
│   │   ├── ohd-storage/             # PKGBUILD + .install + .SRCINFO
│   │   ├── ohd-relay/
│   │   ├── ohd-connect/
│   │   ├── ohd-emergency/
│   │   └── ohd-care/
│   ├── rpm/                         # (cargo-generate-rpm metadata lives in Cargo.toml)
│   ├── assets/
│   │   ├── storage.toml             # default config shipped at /etc/ohd-storage/
│   │   └── relay.toml               # default config shipped at /etc/ohd-relay/
│   └── github-workflows/
│       └── release.yml              # tag-driven CI template
├── storage/crates/ohd-storage-server/Cargo.toml   # [package.metadata.deb] + .generate-rpm
├── relay/Cargo.toml                               # [package.metadata.deb] + .generate-rpm
├── connect/cli/Cargo.toml                         # [package.metadata.deb] + .generate-rpm
├── emergency/cli/Cargo.toml                       # [package.metadata.deb] + .generate-rpm
└── care/cli/PACKAGING.md                          # Python-specific notes
```

## Building

### One-time tooling

```bash
cargo install --locked cargo-deb cargo-generate-rpm
```

### Per-binary

```bash
# Storage (from ohd/storage/)
cargo build --release -p ohd-storage-server
cargo deb           --no-build -p ohd-storage-server   # → target/debian/ohd-storage_*.deb
cargo generate-rpm                  -p ohd-storage-server   # → target/generate-rpm/ohd-storage*.rpm

# Relay (from ohd/relay/)
cargo build --release
cargo deb           --no-build -p ohd-relay
cargo generate-rpm                  -p ohd-relay

# Connect CLI (from ohd/connect/cli/)
cargo build --release
cargo deb           --no-build
cargo generate-rpm

# Emergency CLI (from ohd/emergency/cli/)
cargo build --release
cargo deb           --no-build
cargo generate-rpm

# Arch (from ohd/packaging/arch/<binary>/)
makepkg -si
```

### Care CLI (Python)

See [`care/cli/PACKAGING.md`](care/cli/PACKAGING.md) for the wheel-build
recipe. Native-package wrapping is documented but not fully automated
yet — scope-cut for this pass.

### CI

A GitHub Actions template lives at
[`packaging/github-workflows/release.yml`](packaging/github-workflows/release.yml).
Drop it into a repo's `.github/workflows/` directory; it triggers on
tags matching `<binary>-v<semver>` and uploads .deb + .rpm artifacts to
the release.

The OHD root repo does not currently have a `.github/workflows/`
directory; the template is staged in `packaging/` for the operator to
move into place once the release process formalizes.

## Installing

### Debian / Ubuntu

```bash
# Daemons
sudo dpkg -i ohd-storage_0.1.0_amd64.deb
sudo dpkg -i ohd-relay_0.1.0_amd64.deb

# CLIs
sudo dpkg -i ohd-connect_0.1.0_amd64.deb
sudo dpkg -i ohd-emergency_0.1.0_amd64.deb
```

The .deb's `postinst` creates the `ohd-storage` / `ohd-relay` system
users, sets up `/var/lib/<binary>`, reloads systemd, and enables (but
does not start) the daemon. On purge the user + data dir are removed.

### Fedora / RHEL / openSUSE

```bash
sudo rpm -i ohd-storage-0.1.0-1.x86_64.rpm
sudo rpm -i ohd-relay-0.1.0-1.x86_64.rpm
sudo rpm -i ohd-connect-0.1.0-1.x86_64.rpm
sudo rpm -i ohd-emergency-0.1.0-1.x86_64.rpm
```

The cargo-generate-rpm packages don't ship pre/post scripts in this
pass; the operator has to create the system user manually
(`useradd --system ohd-storage`) before starting the unit. We'll add
RPM scriptlets in a follow-up.

### Arch Linux

```bash
cd packaging/arch/ohd-storage && makepkg -si
cd packaging/arch/ohd-relay   && makepkg -si
cd packaging/arch/ohd-connect && makepkg -si
cd packaging/arch/ohd-emergency && makepkg -si
```

The PKGBUILDs source from a tagged release tarball
(`https://github.com/ohd-foundation/ohd/archive/<binary>-v<ver>.tar.gz`).
For local dev builds, swap `source=()` for a `git+file://` URL pointing
at your working tree. Once the project lands in the AUR, `makepkg -si`
becomes `paru -S ohd-storage` (or any AUR helper).

## After installing

### Storage daemon

```bash
# Initialize the database (one-time):
sudo -u ohd-storage /usr/bin/ohd-storage-server init \
    --db /var/lib/ohd-storage/storage.db

# Issue a self-session token (write it down — there is no recovery):
sudo -u ohd-storage /usr/bin/ohd-storage-server issue-self-token \
    --db /var/lib/ohd-storage/storage.db
# → ohds_…

# Start the service:
sudo systemctl enable --now ohd-storage.service

# Verify:
curl http://localhost:8443/ohdc.v0.OhdcService/Health \
     -H 'Content-Type: application/json' \
     --data '{}'
```

### Relay daemon

```bash
# Edit the config first (push providers, optional authority mode):
sudoedit /etc/ohd-relay/relay.toml

# Then:
sudo systemctl enable --now ohd-relay.service
sudo systemctl status ohd-relay.service
```

## Filesystem layout

| Path | Owner | Mode | Purpose |
|---|---|---|---|
| `/usr/bin/ohd-storage-server` | root:root | 0755 | storage binary |
| `/usr/bin/ohd-relay` | root:root | 0755 | relay binary |
| `/usr/bin/ohd-connect` | root:root | 0755 | connect CLI |
| `/usr/bin/ohd-emergency` | root:root | 0755 | emergency CLI |
| `/lib/systemd/system/ohd-storage.service` | root:root | 0644 | storage unit |
| `/lib/systemd/system/ohd-relay.service` | root:root | 0644 | relay unit |
| `/etc/ohd-storage/storage.toml` | root:ohd-storage | 0640 | storage config (placeholder; see file) |
| `/etc/ohd-relay/relay.toml` | root:ohd-relay | 0640 | relay config |
| `/var/lib/ohd-storage/` | ohd-storage:ohd-storage | 0750 | SQLCipher DB + WAL + sidecar blobs |
| `/var/lib/ohd-relay/` | ohd-relay:ohd-relay | 0750 | registration SQLite |
| `/usr/share/doc/ohd-*/README.md` | root:root | 0644 | per-binary README |
| `/usr/share/licenses/ohd-*/` | root:root | 0644 | LICENSE-APACHE + LICENSE-MIT |

Both daemon units run with extensive systemd hardening
(`ProtectSystem=strict`, `NoNewPrivileges=true`,
`MemoryDenyWriteExecute=true`, `CapabilityBoundingSet=` empty,
`SystemCallFilter=@system-service`). See the unit files for the full
list.

## Comparison to Docker

|  | Docker (existing) | Native packages (this PR) |
|---|---|---|
| Distribution | `ohd-storage:dev` image, compose file | `.deb` / `.rpm` / Arch PKGBUILD |
| Init / setup | `docker compose run` | systemd + maintainer scripts |
| User isolation | UID 10001 inside container | dedicated `ohd-storage` system user |
| TLS | front with Caddy in compose | front with Caddy/nginx (or pass `--http3-cert`) |
| Persistence | named volume `ohd_storage_data` | bind path `/var/lib/ohd-storage` |
| Logs | `docker logs` | `journalctl -u ohd-storage` |
| Upgrade | `docker compose pull && up` | `apt upgrade` / `dnf upgrade` / `pacman -Syu` |
| Best for | dev, demo, container hosts | bare-metal, VMs, home servers |

The two paths share zero implementation surface — Docker is built from
`storage/deploy/Dockerfile`, native packages are built from the same
release binary via Cargo metadata. Both end up at
`/var/lib/ohd-storage/storage.db` (Docker via volume mount, native via
direct path), so a database created under one can be migrated to the
other.

## Status / known gaps

- **P0** (cargo-deb metadata): done for the four Rust binaries.
- **P1** (cargo-generate-rpm metadata): done.
- **P2** (Arch PKGBUILDs): done; checksums stay `SKIP` until release
  tarballs are cut.
- **P3** (systemd units): done with full hardening sandbox.
- **P4** (Debian maintainer scripts): done for storage, relay; care has
  a stub.
- **P5** (`ohd-care` Python wheel + .deb wrapper): documented in
  `care/cli/PACKAGING.md`; not fully automated. PKGBUILD is shipped.
- **P6** (CI): GitHub Actions template at
  `packaging/github-workflows/release.yml`. Not activated — the OHD root
  has no `.github/workflows/` directory yet; move the file into place
  per repo when releases formalize.
- **P7** (this README): done.

### What's not done

- RPM `%pre` / `%post` scriptlets — cargo-generate-rpm supports them via
  `[package.metadata.generate-rpm.post_install_script]` but we punted
  for v0.1; operator has to `useradd --system ohd-storage` manually.
- Cross-compilation actually working in CI — the workflow template is
  written but not exercised. Aarch64 linker name + build deps may need
  tweaking.
- AUR submission (`ohd-storage`, `ohd-relay`, etc. as AUR packages).
  PKGBUILDs are submission-ready in shape; need a maintainer + GPG key.
- Care CLI native wrapping — documented in `care/cli/PACKAGING.md` as
  a manual recipe, not automated.

## Validation

cargo-deb / cargo-generate-rpm aren't installed in the dev environment,
so we couldn't run `cargo deb --no-build -p ohd-storage-server` to
validate metadata syntactically. The metadata follows the documented
schema for both crates as of cargo-deb 2.x and cargo-generate-rpm 0.14;
the most likely failure is a path typo on the cross-tree relative
references (paths into `../packaging/` from each Cargo.toml). When you
install the tools and run a build, the first error will pinpoint any
mistake.
