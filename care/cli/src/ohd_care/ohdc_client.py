"""Hand-rolled Connect-RPC over HTTP/2 (h2c) client for the OHDC service.

Why hand-rolled? `connectrpc` 0.4 is Rust-only; the official Connect Python
runtime targets HTTP/1.1 only and isn't pinned by the workspace. The CLI's
needs are small — a handful of unary calls plus `QueryEvents` server
streaming — so we frame the wire ourselves over httpx. The same approach
the Rust CLI uses for its HTTP/3 fallback (`connect/cli/src/client.rs`,
``H3RawClient``).

Wire shape (per https://connectrpc.com/docs/protocol/):

- **Unary**: `POST /<package>.<Service>/<Method>` with
  ``content-type: application/proto`` and the raw protobuf body. Response
  is the same: HTTP 200 + ``application/proto`` body = raw protobuf
  message; non-2xx = JSON envelope error.
- **Server-streaming**: `POST` with
  ``content-type: application/connect+proto``. The request body is one
  framed envelope: ``[1 byte flags][4 bytes BE length][payload]`` where
  flags=0 = data, payload = proto-encoded request. The response body is
  the same envelope shape, repeated; flags & 0x02 = end-of-stream, payload
  is JSON ({} on success, {"error":{...}} on failure).

Auth is a per-request ``Authorization: Bearer <token>`` header.
"""

from __future__ import annotations

import contextlib
import json
import struct
from collections.abc import Iterator
from dataclasses import dataclass
from typing import Any, TypeVar

import httpx

# Trigger lazy codegen + sys.path tweak before importing the generated
# stubs.
from . import ohdc_proto as _ohdc_proto  # noqa: F401  (side-effect import)
from ohdc.v0 import ohdc_pb2 as pb  # type: ignore[import-not-found]

from google.protobuf.message import Message  # noqa: E402

from .canonical_query_hash import canonical_query_hash  # noqa: E402
from .operator_audit import (  # noqa: E402
    OperatorAuditEntry,
    append_operator_audit_entry,
    build_audit_template,
)

M = TypeVar("M", bound=Message)


# ---------------------------------------------------------------------------
# Filter → canonical-dict mapping
# ---------------------------------------------------------------------------


def _channel_value_to_canonical(cv: Any) -> dict[str, Any]:
    """Translate a ``pb.ChannelScalar``-shaped predicate value into the
    externally-tagged JSON envelope storage hashes (``{"Real": 1.0}`` etc.).

    Storage's ``ChannelScalar`` enum is serialized externally-tagged. The
    operator-side proto's ``ChannelPredicate.value`` is a ``ChannelValue``
    oneof (per the same conventions). We pivot on the active oneof case.
    """
    which = cv.WhichOneof("value") if hasattr(cv, "WhichOneof") else None
    match which:
        case "real_value":
            return {"Real": cv.real_value}
        case "int_value":
            return {"Int": int(cv.int_value)}
        case "bool_value":
            return {"Bool": bool(cv.bool_value)}
        case "text_value":
            return {"Text": cv.text_value}
        case "enum_ordinal":
            return {"EnumOrdinal": int(cv.enum_ordinal)}
        case _:
            return {}


def filter_pb_to_canonical(flt: Any) -> dict[str, Any]:
    """Translate a ``pb.EventFilter`` to the canonical dict shape used by
    :func:`canonical_query_hash`. Only the wire-relevant fields are
    captured. Defaults match the storage Rust struct (``include_superseded``
    defaults to ``true``, ``include_deleted`` to ``false``, etc.).
    """
    if flt is None:
        return {}
    out: dict[str, Any] = {
        "from_ms": flt.from_ms if flt.HasField("from_ms") else None,
        "to_ms": flt.to_ms if flt.HasField("to_ms") else None,
        "event_types_in": list(flt.event_types_in),
        "event_types_not_in": list(flt.event_types_not_in),
        "include_deleted": (
            bool(flt.include_deleted) if flt.HasField("include_deleted") else False
        ),
        "include_superseded": (
            bool(flt.include_superseded)
            if flt.HasField("include_superseded")
            else True
        ),
        "limit": int(flt.limit) if flt.HasField("limit") else None,
        "device_id_in": list(flt.device_id_in),
        "source_in": list(flt.source_in),
        "event_ulids_in": [
            # Storage hashes ULIDs as Crockford strings in the EventFilter
            # JSON; mirror that here. The proto carries them as raw bytes
            # under `ulid.bytes`. v0 callers don't populate this, so we
            # leave it empty rather than re-encode.
        ],
        "sensitivity_classes_in": list(getattr(flt, "sensitivity_classes_in", []) or []),
        "sensitivity_classes_not_in": list(
            getattr(flt, "sensitivity_classes_not_in", []) or []
        ),
        "channel_predicates": [
            {
                "channel_path": p.channel_path,
                "op": p.op,
                "value": _channel_value_to_canonical(p.value)
                if p.HasField("value")
                else {},
            }
            for p in getattr(flt, "channel_predicates", [])
        ],
        "case_ulids_in": [
            # Same caveat as event_ulids_in — Crockford strings on the wire
            # JSON; v0 read paths leave this empty.
        ],
    }
    return out


# ---------------------------------------------------------------------------
# Errors
# ---------------------------------------------------------------------------

class OhdcError(RuntimeError):
    """Base class for OHDC client errors."""


class OhdcAuthError(OhdcError):
    """401 / 403 from the storage server (token rejected, expired, revoked)."""


class OhdcUnimplementedError(OhdcError):
    """The storage server returned `unimplemented` (RPC stubbed in storage)."""


@dataclass(frozen=True)
class OhdcConnectError:
    """Decoded Connect-Protocol error envelope."""

    code: str
    message: str
    raw: dict[str, Any]

    @classmethod
    def from_payload(cls, body: bytes | str) -> "OhdcConnectError":
        if isinstance(body, bytes):
            try:
                body = body.decode("utf-8")
            except UnicodeDecodeError:
                body = body.decode("latin-1")
        try:
            data = json.loads(body) if body else {}
        except json.JSONDecodeError:
            return cls(code="unknown", message=body or "<empty>", raw={})
        # Connect's error envelope is `{"code": "...", "message": "..."}`.
        # In streaming responses it's wrapped: `{"error": {...}}`.
        if isinstance(data, dict) and "error" in data and isinstance(data["error"], dict):
            inner = data["error"]
            return cls(
                code=str(inner.get("code", "unknown")),
                message=str(inner.get("message", "")),
                raw=inner,
            )
        if isinstance(data, dict):
            return cls(
                code=str(data.get("code", "unknown")),
                message=str(data.get("message", body or "")),
                raw=data,
            )
        return cls(code="unknown", message=str(data), raw={})


def _raise_for_connect_error(err: OhdcConnectError, *, rpc: str) -> None:
    """Translate a Connect error envelope into a typed Python exception."""
    msg = f"OHDC {rpc}: {err.code} — {err.message}" if err.message else f"OHDC {rpc}: {err.code}"
    if err.code in ("unauthenticated", "permission_denied"):
        raise OhdcAuthError(msg)
    if err.code == "unimplemented":
        raise OhdcUnimplementedError(msg)
    raise OhdcError(msg)


# ---------------------------------------------------------------------------
# Client
# ---------------------------------------------------------------------------

@dataclass
class OhdcClient:
    """Synchronous OHDC client over HTTP/2 (h2c).

    One client instance owns one ``httpx.Client`` (HTTP/2 enabled) and
    one bearer token. The token is per-request scoped — for Care that's
    a patient's grant token, swapped per call when the active patient
    changes; we expose ``with_token`` for that.
    """

    storage_url: str
    bearer_token: str | None = None
    operator_subject: str | None = None  # OIDC `sub` of the logged-in operator
    _http: httpx.Client | None = None
    _verify: bool | str = True
    _timeout: float = 30.0

    SERVICE = "ohdc.v0.OhdcService"

    def __post_init__(self) -> None:
        if self._http is None:
            # HTTP/2 needs `h2`; the `httpx[http2]` extra installs it. We
            # enable HTTP/2 unconditionally — it works for both `http://`
            # h2c (via prior knowledge) and TLS h2.
            self._http = httpx.Client(
                http2=True,
                base_url=self.storage_url,
                timeout=self._timeout,
                verify=self._verify,
            )

    # -------- lifecycle ----------------------------------------------

    def close(self) -> None:
        if self._http is not None:
            self._http.close()
            self._http = None

    def __enter__(self) -> "OhdcClient":
        return self

    def __exit__(self, exc_type: object, exc: object, tb: object) -> None:
        self.close()

    # -------- low-level wire ----------------------------------------

    def _headers(self, *, content_type: str) -> dict[str, str]:
        h = {
            "content-type": content_type,
            "connect-protocol-version": "1",
            "user-agent": "ohd-care-cli/0.1",
        }
        if self.bearer_token:
            h["authorization"] = f"Bearer {self.bearer_token}"
        # Attach the operator's OIDC subject so the storage's two-sided
        # audit can join "which clinician at the operator" with the
        # grant-token-side audit row. Per spec/care-auth.md "Two-sided
        # audit". Storage ignores the header today; once storage wires
        # its operator-binding it'll pick it up. This is the integration
        # point for `oidc_subject` from `oidc-login`.
        if self.operator_subject:
            h["x-ohd-operator-subject"] = self.operator_subject
        return h

    def _unary(self, method: str, request: Message, response_cls: type[M]) -> M:
        """Send a unary Connect-Protocol POST and decode the response."""
        url = f"/{self.SERVICE}/{method}"
        body = request.SerializeToString()
        assert self._http is not None
        try:
            resp = self._http.post(
                url,
                content=body,
                headers=self._headers(content_type="application/proto"),
            )
        except httpx.HTTPError as exc:
            raise OhdcError(f"OHDC {method}: transport error — {exc}") from exc

        if resp.status_code != 200:
            err = OhdcConnectError.from_payload(resp.content)
            _raise_for_connect_error(err, rpc=method)

        msg = response_cls()
        try:
            msg.ParseFromString(resp.content)
        except Exception as exc:  # protobuf decode error
            raise OhdcError(
                f"OHDC {method}: failed to decode response ({exc}); "
                f"first bytes={resp.content[:64]!r}"
            ) from exc
        return msg

    def _server_stream(
        self, method: str, request: Message, response_cls: type[M]
    ) -> Iterator[M]:
        """Server-streaming Connect-Protocol POST. Yields decoded responses."""
        url = f"/{self.SERVICE}/{method}"
        # Wrap the request in a single Connect data frame.
        req_body = request.SerializeToString()
        framed = b"\x00" + struct.pack(">I", len(req_body)) + req_body
        headers = self._headers(content_type="application/connect+proto")
        assert self._http is not None
        try:
            with self._http.stream(
                "POST",
                url,
                content=framed,
                headers=headers,
            ) as resp:
                if resp.status_code != 200:
                    body = resp.read()
                    err = OhdcConnectError.from_payload(body)
                    _raise_for_connect_error(err, rpc=method)
                yield from _decode_connect_stream(resp.iter_bytes(), response_cls, rpc=method)
        except httpx.HTTPError as exc:
            raise OhdcError(f"OHDC {method}: transport error — {exc}") from exc

    # -------- token rotation -----------------------------------------

    @contextlib.contextmanager
    def with_token(self, token: str) -> Iterator["OhdcClient"]:
        """Temporarily swap the bearer token for a series of calls."""
        old = self.bearer_token
        self.bearer_token = token
        try:
            yield self
        finally:
            self.bearer_token = old

    # -------- audit helpers ------------------------------------------

    def _audit_template(
        self,
        *,
        ohdc_action: str,
        query_kind: str | None,
        query_hash_hex: str | None,
    ) -> OperatorAuditEntry:
        """Pre-baked audit row; finished by :meth:`_audit_finish`."""
        return build_audit_template(
            ohdc_action=ohdc_action,
            query_kind=query_kind,
            query_hash_hex=query_hash_hex,
            grant_ulid="",  # set by callers that know it (v0 doesn't yet)
            operator_subject=self.operator_subject,
        )

    def _audit_finish(
        self,
        template: OperatorAuditEntry,
        *,
        result: str,
        rows_returned: int | None = None,
        rows_filtered: int | None = None,
        reason: str | None = None,
    ) -> None:
        # Re-stamp the template with the resolved outcome and persist.
        template.result = result  # type: ignore[assignment]
        template.rows_returned = rows_returned
        template.rows_filtered = rows_filtered
        template.reason = reason
        append_operator_audit_entry(template)

    # -------- high-level RPCs ----------------------------------------

    def health(self) -> "pb.HealthResponse":
        return self._unary("Health", pb.HealthRequest(), pb.HealthResponse)

    def who_am_i(self) -> "pb.WhoAmIResponse":
        return self._unary("WhoAmI", pb.WhoAmIRequest(), pb.WhoAmIResponse)

    def put_events(self, req: "pb.PutEventsRequest") -> "pb.PutEventsResponse":
        return self._unary("PutEvents", req, pb.PutEventsResponse)

    def query_events(self, req: "pb.QueryEventsRequest") -> Iterator["pb.Event"]:
        # Compute the canonical query-hash + open an operator-side audit row
        # **before** the call so we record the row even when storage rejects
        # or errors out (per `care/SPEC.md` §7.3 the patient-side audit row
        # already exists; the operator side is what we control).
        canonical = filter_pb_to_canonical(req.filter) if req.HasField("filter") else {}
        query_hash = canonical_query_hash("query_events", canonical)
        template = self._audit_template(
            ohdc_action="query_events",
            query_kind="query_events",
            query_hash_hex=query_hash,
        )
        rows: list[pb.Event] = []
        try:
            for ev in self._server_stream("QueryEvents", req, pb.Event):
                rows.append(ev)
                yield ev
        except OhdcError as exc:
            self._audit_finish(template, result="error", reason=str(exc))
            raise
        else:
            self._audit_finish(template, result="success", rows_returned=len(rows))

    def get_event_by_ulid(self, req: "pb.GetEventByUlidRequest") -> "pb.Event":
        # `get_event_by_ulid` is a single-row lookup; storage's pending-query
        # path keys on `query_kind="get_event_by_ulid"` with an empty filter.
        # We mirror that so the operator audit JOINs cleanly.
        query_hash = canonical_query_hash("get_event_by_ulid", {})
        template = self._audit_template(
            ohdc_action="get_event_by_ulid",
            query_kind="get_event_by_ulid",
            query_hash_hex=query_hash,
        )
        try:
            resp = self._unary("GetEventByUlid", req, pb.Event)
        except OhdcError as exc:
            self._audit_finish(template, result="error", reason=str(exc))
            raise
        self._audit_finish(template, result="success", rows_returned=1)
        return resp

    def list_pending(self, req: "pb.ListPendingRequest") -> "pb.ListPendingResponse":
        return self._unary("ListPending", req, pb.ListPendingResponse)

    def approve_pending(
        self, req: "pb.ApprovePendingRequest"
    ) -> "pb.ApprovePendingResponse":
        return self._unary("ApprovePending", req, pb.ApprovePendingResponse)

    def reject_pending(
        self, req: "pb.RejectPendingRequest"
    ) -> "pb.RejectPendingResponse":
        return self._unary("RejectPending", req, pb.RejectPendingResponse)

    def list_grants(self, req: "pb.ListGrantsRequest") -> "pb.ListGrantsResponse":
        return self._unary("ListGrants", req, pb.ListGrantsResponse)

    def audit_query(self, req: "pb.AuditQueryRequest") -> Iterator["pb.AuditEntry"]:
        # Server-streaming; storage stub returns Unimplemented today.
        return self._server_stream("AuditQuery", req, pb.AuditEntry)


# ---------------------------------------------------------------------------
# Stream framing helpers
# ---------------------------------------------------------------------------

def _decode_connect_stream(
    chunks: Iterator[bytes],
    message_cls: type[M],
    *,
    rpc: str,
) -> Iterator[M]:
    """Yield decoded messages from a Connect server-streaming body.

    Body format (repeated): ``[1B flags][4B BE length][payload]``. When
    flags & 0x02 == 1 the payload is a JSON envelope marking end-of-stream
    (with optional ``"error"`` for per-stream failure).
    """
    buf = bytearray()
    for chunk in chunks:
        if chunk:
            buf.extend(chunk)
        # Consume as many full frames as we have. Connect frames have a
        # 5-byte header; if we don't have a header yet, wait for more.
        while True:
            if len(buf) < 5:
                break
            flags = buf[0]
            length = struct.unpack_from(">I", buf, 1)[0]
            if len(buf) < 5 + length:
                break
            payload = bytes(buf[5 : 5 + length])
            del buf[: 5 + length]
            if flags & 0x02:
                # End-of-stream. Surface an error envelope if present.
                err = OhdcConnectError.from_payload(payload)
                if err.code != "unknown" or err.message:
                    # Only raise if the envelope explicitly carries an error;
                    # `{}` decodes to (unknown, "") and means "clean end".
                    if err.raw:
                        _raise_for_connect_error(err, rpc=rpc)
                return
            msg = message_cls()
            try:
                msg.ParseFromString(payload)
            except Exception as exc:
                raise OhdcError(
                    f"OHDC {rpc}: failed to decode stream frame ({exc})"
                ) from exc
            yield msg
    # Stream closed before an end-of-stream frame arrived. The server
    # may have hung up mid-stream; surface what we got and return.
    if buf:
        # Partial frame at EOF — diagnostic hint.
        raise OhdcError(
            f"OHDC {rpc}: stream truncated ({len(buf)} bytes left in buffer)"
        )
