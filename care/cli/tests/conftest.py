"""Pytest config — defaults for the OHD Care CLI test suite.

We force the credential vault into the `none` KMS backend (passthrough,
plaintext envelope) for the whole suite. The reasons:

- Test runners are headless: there's no Secret Service / Keychain /
  Credential Manager to talk to. The `keyring` backend would raise
  `KmsBackendUnavailable` immediately.
- The passphrase fallback would prompt on stdin, which `click.testing`
  doesn't drive. Argon2id-equivalent KDFs are also slow (~100ms each)
  which would inflate the suite duration.
- The real CLI's behaviour (auto-pick keyring then passphrase) is
  exercised by `tests/test_kms.py` directly, with fakes injected.

Tests that want to opt back into a real backend can set their own
``OHD_CARE_KMS_BACKEND`` via `monkeypatch.setenv`.
"""

from __future__ import annotations

import os

# Set on module import so collection-time helpers (which sometimes import
# the CLI's commands and trigger select_backend on the way in) see the
# override too.
os.environ.setdefault("OHD_CARE_KMS_BACKEND", "none")
