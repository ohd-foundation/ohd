"""Active-case vault for the Emergency MCP.

Analogous to Care MCP's grant vault, but keyed by case_id rather than
patient label — emergencies are case-shaped (one paramedic crew, one
patient encounter, one bounded grant). Per ``emergency/SPEC.md`` §3.1:
``set_active_case(case_id)`` is the only tool that changes active context.
"""

from __future__ import annotations

from dataclasses import dataclass

from .config import CaseGrant


class CaseVaultError(RuntimeError):
    pass


class NoActiveCaseError(CaseVaultError):
    def __init__(self) -> None:
        super().__init__(
            "No active case. Call set_active_case(case_id) first; the operator "
            "selects the case from the dispatch console / paramedic tablet."
        )


class UnknownCaseError(CaseVaultError):
    def __init__(self, case_id: str) -> None:
        super().__init__(
            f"No grant for case {case_id!r}. The case must be issued from the "
            "patient phone (break-glass) or via a relay-issued reopen token."
        )


@dataclass
class CaseVault:
    cases: dict[str, CaseGrant]
    active_case_id: str | None = None

    @classmethod
    def from_list(cls, cases: list[CaseGrant]) -> "CaseVault":
        return cls(cases={c.case_id: c for c in cases}, active_case_id=None)

    def list_cases(self) -> list[dict[str, str | None | bool]]:
        return [
            {
                "case_id": c.case_id,
                "label": c.label,
                "active": c.case_id == self.active_case_id,
            }
            for c in self.cases.values()
        ]

    def set_active(self, case_id: str) -> CaseGrant:
        if case_id not in self.cases:
            raise UnknownCaseError(case_id)
        self.active_case_id = case_id
        return self.cases[case_id]

    def current(self) -> CaseGrant | None:
        if self.active_case_id is None:
            return None
        return self.cases[self.active_case_id]

    def require_current(self) -> CaseGrant:
        active = self.current()
        if active is None:
            raise NoActiveCaseError()
        return active
