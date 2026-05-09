# OHD Care — CLI

`ohd-care` — terminal interface for the OHD Care reference clinical app. Speaks OHDC under grant-token auth against any patient's storage instance.

See [`STATUS.md`](STATUS.md) for the per-subcommand wire state. The full subcommand surface (`patients`, `use`, `temperature`, `submit`, `pending`, …) is exercised in `tests/`.

## Stack

- **Python ≥ 3.11**
- **Click** — command framework. Composable groups map to the subcommand hierarchy in `SPEC.md` §11.
- **Hatchling** — build backend.
- **uv** — recommended package runner (faster than `pip`); `pip` works too.
- **`ohd-shared`** workspace package — proto stubs, transport, canonical query hash, OAuth helpers (declared via `[tool.uv.sources]`).

## Install

```sh
# uv (recommended)
uv sync                    # creates .venv, installs deps + ohd-shared

# or, with vanilla pip
python -m venv .venv
. .venv/bin/activate
pip install -e ../../packages/python/ohd-shared
pip install -e ".[dev]"
```

A console script `ohd-care` is registered via `[project.scripts]`.

## Run

```sh
ohd-care --help
ohd-care --version
ohd-care login --storage https://ohd.example.com
ohd-care patients
ohd-care use alice
ohd-care temperature --last-72h
ohd-care submit observation --type=respiratory_rate --value=18
ohd-care submit clinical-note --about="visit 2026-05-07" < notes.txt
ohd-care pending list
```

## Test

```sh
uv run pytest
```

The suite covers the click command tree, OIDC flow, OHDC client wiring, and the operator-side audit writer. Pure-Python; doesn't require an external OHDC backend (uses an in-process fake).

## Lint / format

```sh
uv run ruff check src tests
uv run ruff format src tests
```

## Packaging

For native distro packaging (Python wheel + .deb wrapper + Arch PKGBUILD), see [`PACKAGING.md`](PACKAGING.md). Top-level pointer: [`../../PACKAGING.md`](../../PACKAGING.md).

## See also

- [`../SPEC.md`](../SPEC.md) — full implementation contract.
- [`../STATUS.md`](../STATUS.md) — status of every subcommand.
- [`../demo/run.sh`](../demo/run.sh) — end-to-end demo.

## License

Dual-licensed `Apache-2.0 OR MIT`, matching the project root.
