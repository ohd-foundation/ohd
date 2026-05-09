"""In-memory grant vault — the multi-patient state machine.

Per ``care/SPEC.md`` §10.6: the active patient must appear in every tool
result for orientation, and ``switch_patient`` is the **only** tool that
changes active context. This module enforces those invariants.

For v0 the vault is in-memory, seeded from a config file (see
``config.CareMcpConfig.grants``). Persistence (encrypted at rest with
deployment KMS, per SPEC §14) is the wire-up agent's job.
"""

from __future__ import annotations

from dataclasses import dataclass

from .config import PatientGrant


class GrantVaultError(RuntimeError):
    """Base class for vault errors surfaced to the LLM."""


class NoActivePatientError(GrantVaultError):
    """Raised when a per-patient tool is called without ``switch_patient`` first."""

    def __init__(self) -> None:
        super().__init__(
            "No active patient. Call switch_patient(label) first; use "
            "list_patients() to see available labels."
        )


class UnknownPatientError(GrantVaultError):
    def __init__(self, label: str) -> None:
        super().__init__(
            f"No grant for patient label {label!r}. Use list_patients() to see "
            "available labels."
        )


@dataclass
class GrantVault:
    """Operator-side grant vault, keyed by patient label."""

    grants: dict[str, PatientGrant]
    active_label: str | None = None

    @classmethod
    def from_list(cls, grants: list[PatientGrant]) -> "GrantVault":
        return cls(grants={g.label: g for g in grants}, active_label=None)

    def list_patients(self) -> list[dict[str, str | None]]:
        return [
            {
                "label": g.label,
                "scope_summary": g.scope_summary,
                "active": g.label == self.active_label,
            }
            for g in self.grants.values()
        ]

    def switch(self, label: str) -> PatientGrant:
        if label not in self.grants:
            raise UnknownPatientError(label)
        self.active_label = label
        return self.grants[label]

    def current(self) -> PatientGrant | None:
        if self.active_label is None:
            return None
        return self.grants[self.active_label]

    def require_current(self) -> PatientGrant:
        active = self.current()
        if active is None:
            raise NoActivePatientError()
        return active
