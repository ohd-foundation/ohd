"""KMS abstraction for credential vaults.

Per ``../../spec/care-auth.md`` "Token storage on the Care server
(encryption)" — Care MUST encrypt grant tokens at rest. The CLI is the
solo-practitioner shape (see "PBKDF2-derived local key file (root-
readable) for self-hosted-by-practitioner Care") so we ship two
backends and a `none` opt-out:

- **`keyring`** — uses the OS secret store (Linux Secret Service / libsecret,
  macOS Keychain, Windows Credential Manager) via the `keyring` PyPI
  package. Default; best DX. Falls through to `passphrase` on headless
  machines (CI, Docker without a session bus).
- **`passphrase`** — derives a per-vault key from a user-supplied
  passphrase via `argon2id` (using the `cryptography` package's
  scrypt fallback, since `argon2-cffi` isn't a direct dependency yet —
  scrypt is ROM-hard and safe at the chosen parameters). Encrypts the
  payload with AES-GCM. The passphrase can also be supplied via the
  `OHD_CARE_VAULT_PASSPHRASE` env var for unattended runs.
- **`none`** — opt-out for tests and the on-disk legacy mode. The
  payload is stored as plaintext, mode 0600. Equivalent to the
  pre-KMS shipping shape.

Wire format on disk (the credentials file becomes a thin envelope):

```
{
  "version": 1,
  "kms": "keyring" | "passphrase" | "none",
  "ciphertext_b64": "...",        # absent if kms == "none"
  "plaintext_toml": "...",        # absent unless kms == "none"
  "kms_meta": { ... }             # backend-specific, e.g. salt + nonce for passphrase
}
```

The actual payload (the operator credentials TOML) is round-tripped
through ``encrypt(plaintext_bytes) -> envelope_dict`` /
``decrypt(envelope_dict) -> plaintext_bytes``. Callers stay agnostic
to which backend ran.
"""

from __future__ import annotations

import base64
import json
import os
import secrets
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Protocol


VAULT_FORMAT_VERSION = 1


class KmsError(RuntimeError):
    """Base class for KMS backend failures."""


class KmsBackendUnavailable(KmsError):
    """Selected backend can't initialize (e.g. headless machine, no Secret Service)."""


class KmsDecryptError(KmsError):
    """Ciphertext can't be decrypted with the available material."""


class KmsBackend(Protocol):
    """Protocol every backend implements.

    The "envelope" returned by `encrypt` is JSON-serializable; callers
    write it to disk as-is. The plaintext is bytes (callers know the
    payload schema, e.g. TOML).
    """

    name: str

    def encrypt(self, plaintext: bytes) -> dict[str, Any]: ...
    def decrypt(self, envelope: dict[str, Any]) -> bytes: ...


# ---------------------------------------------------------------------------
# Backend: none (passthrough, for tests / legacy mode)
# ---------------------------------------------------------------------------

@dataclass
class NoneBackend:
    """Passthrough — the payload is stored verbatim."""

    name: str = "none"

    def encrypt(self, plaintext: bytes) -> dict[str, Any]:
        return {
            "version": VAULT_FORMAT_VERSION,
            "kms": self.name,
            "plaintext_b64": base64.b64encode(plaintext).decode("ascii"),
        }

    def decrypt(self, envelope: dict[str, Any]) -> bytes:
        if envelope.get("kms") != self.name:
            raise KmsDecryptError(
                f"NoneBackend cannot decrypt envelope with kms={envelope.get('kms')!r}"
            )
        b64 = envelope.get("plaintext_b64")
        if not isinstance(b64, str):
            raise KmsDecryptError("NoneBackend envelope missing `plaintext_b64`")
        try:
            return base64.b64decode(b64.encode("ascii"))
        except (ValueError, TypeError) as exc:
            raise KmsDecryptError(f"NoneBackend: malformed base64: {exc}") from exc


# ---------------------------------------------------------------------------
# Backend: keyring (OS secret store)
# ---------------------------------------------------------------------------

# We use the OS keyring to store an opaque per-vault encryption key (32
# random bytes). The on-disk envelope holds AES-GCM(plaintext) under
# that key. This indirection means rotating the vault key doesn't
# require re-prompting the OS keychain for every operation, and we can
# preserve the same data-at-rest format whether the user is on
# keyring or passphrase.

_KEYRING_SERVICE = "ohd-care.cli"
_KEYRING_USER = "vault-key-v1"


@dataclass
class KeyringBackend:
    """Backend that wraps an AES-GCM key cached in the OS keyring."""

    name: str = "keyring"
    _service: str = _KEYRING_SERVICE
    _user: str = _KEYRING_USER

    def _get_or_create_key(self) -> bytes:
        try:
            import keyring
            import keyring.errors as keyring_errors
        except ImportError as exc:
            raise KmsBackendUnavailable(
                "keyring package not installed (uv add keyring)"
            ) from exc
        try:
            existing = keyring.get_password(self._service, self._user)
        except keyring_errors.KeyringError as exc:
            raise KmsBackendUnavailable(
                f"OS keyring unavailable: {type(exc).__name__}: {exc}"
            ) from exc
        if existing:
            try:
                key = base64.b64decode(existing.encode("ascii"))
            except (ValueError, TypeError) as exc:
                raise KmsBackendUnavailable(f"keyring: malformed stored key: {exc}") from exc
            if len(key) != 32:
                raise KmsBackendUnavailable(
                    f"keyring: stored key has length {len(key)}, expected 32"
                )
            return key
        new_key = secrets.token_bytes(32)
        try:
            keyring.set_password(
                self._service,
                self._user,
                base64.b64encode(new_key).decode("ascii"),
            )
        except keyring_errors.KeyringError as exc:
            raise KmsBackendUnavailable(
                f"OS keyring set failed: {type(exc).__name__}: {exc}"
            ) from exc
        return new_key

    def encrypt(self, plaintext: bytes) -> dict[str, Any]:
        from cryptography.hazmat.primitives.ciphers.aead import AESGCM

        key = self._get_or_create_key()
        nonce = secrets.token_bytes(12)
        aead = AESGCM(key)
        ct = aead.encrypt(nonce, plaintext, b"ohd-care.vault.v1")
        return {
            "version": VAULT_FORMAT_VERSION,
            "kms": self.name,
            "ciphertext_b64": base64.b64encode(ct).decode("ascii"),
            "kms_meta": {
                "nonce_b64": base64.b64encode(nonce).decode("ascii"),
                "aead": "AES-GCM",
                "service": self._service,
                "user": self._user,
            },
        }

    def decrypt(self, envelope: dict[str, Any]) -> bytes:
        from cryptography.hazmat.primitives.ciphers.aead import AESGCM
        from cryptography.exceptions import InvalidTag

        if envelope.get("kms") != self.name:
            raise KmsDecryptError(
                f"KeyringBackend cannot decrypt envelope with kms={envelope.get('kms')!r}"
            )
        meta = envelope.get("kms_meta") or {}
        ct_b64 = envelope.get("ciphertext_b64")
        nonce_b64 = meta.get("nonce_b64")
        if not isinstance(ct_b64, str) or not isinstance(nonce_b64, str):
            raise KmsDecryptError("KeyringBackend envelope missing ciphertext/nonce")
        try:
            ct = base64.b64decode(ct_b64.encode("ascii"))
            nonce = base64.b64decode(nonce_b64.encode("ascii"))
        except (ValueError, TypeError) as exc:
            raise KmsDecryptError(f"KeyringBackend malformed base64: {exc}") from exc
        key = self._get_or_create_key()
        aead = AESGCM(key)
        try:
            return aead.decrypt(nonce, ct, b"ohd-care.vault.v1")
        except InvalidTag as exc:
            raise KmsDecryptError(
                "KeyringBackend AEAD tag mismatch (key may have rotated)"
            ) from exc


# ---------------------------------------------------------------------------
# Backend: passphrase (Argon2id-equivalent via scrypt + AES-GCM)
# ---------------------------------------------------------------------------

@dataclass
class PassphraseBackend:
    """Backend that derives the AES-GCM key from a user passphrase.

    Uses scrypt(N=2**15, r=8, p=1) — RFC 7914-ish parameters that take
    ~100ms on a modern desktop, enough to slow down an offline brute
    force without hurting interactive UX. Argon2id would be marginally
    better but pulls in another C dependency; scrypt is in
    `cryptography` already.
    """

    name: str = "passphrase"
    _passphrase_provider: "callable[[], str] | None" = None

    # Tunable parameters (kept as instance attrs so tests can dial down).
    n: int = 2**15
    r: int = 8
    p: int = 1
    dklen: int = 32

    def _get_passphrase(self, *, prompting_for: str = "vault") -> str:
        env = os.environ.get("OHD_CARE_VAULT_PASSPHRASE")
        if env:
            return env
        if self._passphrase_provider is not None:
            return self._passphrase_provider()
        # Fall back to prompting on stdin (interactive CLI use).
        import getpass

        return getpass.getpass(f"OHD Care {prompting_for} passphrase: ")

    def _derive(self, passphrase: str, salt: bytes) -> bytes:
        from cryptography.hazmat.primitives.kdf.scrypt import Scrypt

        kdf = Scrypt(salt=salt, length=self.dklen, n=self.n, r=self.r, p=self.p)
        return kdf.derive(passphrase.encode("utf-8"))

    def encrypt(self, plaintext: bytes) -> dict[str, Any]:
        from cryptography.hazmat.primitives.ciphers.aead import AESGCM

        passphrase = self._get_passphrase(prompting_for="new vault")
        salt = secrets.token_bytes(16)
        nonce = secrets.token_bytes(12)
        key = self._derive(passphrase, salt)
        aead = AESGCM(key)
        ct = aead.encrypt(nonce, plaintext, b"ohd-care.vault.v1")
        return {
            "version": VAULT_FORMAT_VERSION,
            "kms": self.name,
            "ciphertext_b64": base64.b64encode(ct).decode("ascii"),
            "kms_meta": {
                "salt_b64": base64.b64encode(salt).decode("ascii"),
                "nonce_b64": base64.b64encode(nonce).decode("ascii"),
                "kdf": "scrypt",
                "n": self.n,
                "r": self.r,
                "p": self.p,
                "aead": "AES-GCM",
            },
        }

    def decrypt(self, envelope: dict[str, Any]) -> bytes:
        from cryptography.hazmat.primitives.ciphers.aead import AESGCM
        from cryptography.exceptions import InvalidTag

        if envelope.get("kms") != self.name:
            raise KmsDecryptError(
                f"PassphraseBackend cannot decrypt envelope with kms={envelope.get('kms')!r}"
            )
        meta = envelope.get("kms_meta") or {}
        ct_b64 = envelope.get("ciphertext_b64")
        salt_b64 = meta.get("salt_b64")
        nonce_b64 = meta.get("nonce_b64")
        if not all(isinstance(x, str) for x in (ct_b64, salt_b64, nonce_b64)):
            raise KmsDecryptError("PassphraseBackend envelope missing ciphertext/salt/nonce")
        try:
            ct = base64.b64decode(ct_b64.encode("ascii"))  # type: ignore[union-attr]
            salt = base64.b64decode(salt_b64.encode("ascii"))  # type: ignore[union-attr]
            nonce = base64.b64decode(nonce_b64.encode("ascii"))  # type: ignore[union-attr]
        except (ValueError, TypeError) as exc:
            raise KmsDecryptError(f"PassphraseBackend malformed base64: {exc}") from exc
        passphrase = self._get_passphrase(prompting_for="vault")
        key = self._derive(passphrase, salt)
        aead = AESGCM(key)
        try:
            return aead.decrypt(nonce, ct, b"ohd-care.vault.v1")
        except InvalidTag as exc:
            raise KmsDecryptError("PassphraseBackend: wrong passphrase or tampered file") from exc


# ---------------------------------------------------------------------------
# Selection / factory
# ---------------------------------------------------------------------------

VALID_BACKENDS = ("keyring", "passphrase", "none")


def select_backend(name: str = "auto") -> KmsBackend:
    """Pick a backend by name. ``"auto"`` tries keyring then passphrase.

    The ``OHD_CARE_KMS_BACKEND`` env var overrides the requested value
    when it's non-empty — useful for locking CI to a deterministic
    backend without threading a flag through every CLI subcommand.
    """
    override = os.environ.get("OHD_CARE_KMS_BACKEND", "").strip()
    if override:
        name = override
    if name == "auto":
        try:
            backend = KeyringBackend()
            backend._get_or_create_key()
            return backend
        except KmsBackendUnavailable:
            return PassphraseBackend()
    if name == "keyring":
        return KeyringBackend()
    if name == "passphrase":
        return PassphraseBackend()
    if name == "none":
        return NoneBackend()
    raise KmsError(f"unknown KMS backend {name!r}; expected one of: auto, {VALID_BACKENDS}")


def detect_envelope_backend(envelope: dict[str, Any]) -> str:
    """Read the ``kms`` field from an envelope, with a sensible default."""
    kms = envelope.get("kms")
    if isinstance(kms, str) and kms in VALID_BACKENDS:
        return kms
    # Legacy files (raw TOML, no envelope) are handled by callers.
    raise KmsError(f"envelope has unknown kms backend: {kms!r}")


# ---------------------------------------------------------------------------
# Envelope <-> JSON file helpers
# ---------------------------------------------------------------------------

def write_envelope_file(path: Path, envelope: dict[str, Any]) -> None:
    """Write an envelope dict to ``path`` as JSON, mode 0600, atomically."""
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    body = json.dumps(envelope, indent=2, sort_keys=True).encode("utf-8")
    flags = os.O_WRONLY | os.O_CREAT | os.O_TRUNC
    fd = os.open(tmp, flags, 0o600)
    try:
        os.write(fd, body)
    finally:
        os.close(fd)
    os.replace(tmp, path)
    try:
        os.chmod(path, 0o600)
    except OSError:
        pass


def read_envelope_file(path: Path) -> dict[str, Any]:
    """Read a JSON envelope, or raise :class:`KmsError`."""
    raw = path.read_text()
    try:
        envelope = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise KmsError(f"failed to parse vault envelope at {path}: {exc}") from exc
    if not isinstance(envelope, dict):
        raise KmsError(f"vault envelope at {path} is not a JSON object")
    return envelope
