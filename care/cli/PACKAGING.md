# OHD Care CLI — packaging notes

`ohd-care` is a Python project (see `pyproject.toml`). Native distro
packaging is a thin wrapper around the upstream wheel; we deliberately
avoid the heavyweight `dh-virtualenv` route because the deps are small
and pure-Python except for `protobuf`, which has manylinux wheels.

This is the Python-specific note. The cross-binary packaging tree (systemd
units, .deb metadata, Arch PKGBUILDs for the four Rust binaries) lives at
[`../../PACKAGING.md`](../../PACKAGING.md).

## Building the wheel

```bash
cd care/cli
python3 -m pip install --upgrade build
python3 -m build --wheel
# → care/cli/dist/ohd_care-0.1.0-py3-none-any.whl
```

## Building the .deb

The .deb is a wrapper: it ships the wheel + dependency wheels under
`/usr/lib/ohd-care/wheels`, plus a launcher at `/usr/bin/ohd-care`. The
postinst (`packaging/debian/ohd-care/postinst`) calls `pip install` from
the bundled wheels into `/usr/lib/ohd-care`.

We stop short of a fully-templated build flow today; the canonical path
is:

```bash
# 1. Build the wheel + collect deps
cd care/cli
python3 -m build --wheel
python3 -m pip download \
    --dest dist/wheels \
    --requirement <(python3 -c 'import tomllib; print("\n".join(tomllib.load(open("pyproject.toml","rb"))["project"]["dependencies"]))')

# 2. Hand-build the .deb structure
mkdir -p deb-build/usr/bin
mkdir -p deb-build/usr/lib/ohd-care/wheels
mkdir -p deb-build/DEBIAN
cp dist/*.whl deb-build/usr/lib/ohd-care/wheels/
cp dist/wheels/*.whl deb-build/usr/lib/ohd-care/wheels/
cp ../../packaging/debian/ohd-care/postinst deb-build/DEBIAN/postinst
cp ../../packaging/debian/ohd-care/postrm deb-build/DEBIAN/postrm
chmod 0755 deb-build/DEBIAN/postinst deb-build/DEBIAN/postrm

# 3. Launcher
cat > deb-build/usr/bin/ohd-care <<'EOF'
#!/bin/sh
exec python3 -m ohd_care.cli "$@"
EOF
chmod 0755 deb-build/usr/bin/ohd-care

# 4. control file (paste the snippet below into deb-build/DEBIAN/control)
# 5. dpkg-deb --build deb-build ohd-care_0.1.0_all.deb
```

### `DEBIAN/control` template

```
Package: ohd-care
Version: 0.1.0
Section: net
Priority: optional
Architecture: all
Depends: python3 (>= 3.11), python3-pip
Maintainer: OHD Project <maintainers@ohd.example>
Description: OHD Care CLI — terminal interface for the OHD reference clinical app.
 Connects to a patient's OHD Storage via OHDC under a grant token,
 surfaces clinical-note timelines, and writes notes back. Targets the
 same wire surface as the OHD Care web app and MCP server.
```

## Future: `dh-virtualenv` path

If we want a fully-isolated install (Care wheel in a private venv with
its own Python), `dh-virtualenv` is the standard Debian tool. Cost:
larger .deb (~50 MB vs. ~5 MB) and a Debian-only build. We'll revisit
once the Care CLI matures past the demo stage.

## RPM + Arch

Both reduce to the same shape: build the wheel, package it, install via
`pip install --no-index --find-links <wheels>`. A working PKGBUILD
stub lives at `packaging/arch/ohd-care/PKGBUILD`.
