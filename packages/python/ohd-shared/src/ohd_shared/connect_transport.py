"""Hand-rolled Connect-RPC over HTTP/2 transport (shared).

We don't depend on a Connect-RPC Python runtime. The Connect protocol's
unary path is small enough to implement directly over ``httpx`` (~50 LOC):

- POST ``{base_url}/<package>.<Service>/<Method>``
- Headers: ``Content-Type: application/proto``, ``Connect-Protocol-Version: 1``,
  optional ``Authorization: Bearer <token>``
- Body: protobuf-binary serialized request message
- Response 200: protobuf-binary serialized response message
- Response non-200: JSON error envelope ``{"code": "...", "message": "...", "details": [...]}``

Server-streaming (used by ``QueryEvents``, ``AuditQuery``) uses Connect's
``application/connect+proto`` envelope: a sequence of
5-byte prefixed frames, where the prefix is one ``flags`` byte
(``0x02`` for the trailing end-stream message) and a 4-byte big-endian
length. The trailing frame's payload is JSON ``{"error"?: ..., "metadata"?: ...}``.

References: https://connectrpc.com/docs/protocol/

This module replaces the per-MCP ``_connect_transport.py`` copies that
previously lived in ``care/mcp/``, ``connect/mcp/`` and ``emergency/mcp/``.

Public surface:

- :class:`OhdcTransport` — owns the ``httpx.AsyncClient`` and dispatches
  unary + server-streaming RPCs. The ``Authorization`` header is supplied
  per call so the same transport serves multiple grant tokens (Care,
  Emergency).
- :class:`OhdcRpcError` — raised on Connect error envelopes; carries the
  Connect ``code`` (e.g. ``"unauthenticated"``, ``"out_of_range"``) and
  the human ``message``.
"""

from __future__ import annotations

import json
import struct
from collections.abc import AsyncIterator
from typing import TYPE_CHECKING

import httpx

if TYPE_CHECKING:
    from google.protobuf.message import Message


class OhdcRpcError(Exception):
    """Connect-RPC error envelope.

    Attributes:
        code: Connect canonical error code, e.g. ``"unauthenticated"``,
            ``"permission_denied"``, ``"not_found"``, ``"unimplemented"``.
        message: Human-readable message.
        http_status: The HTTP status code that carried the envelope.
    """

    def __init__(self, code: str, message: str, http_status: int) -> None:
        super().__init__(f"OHDC RPC error [{code}] {message}")
        self.code = code
        self.message = message
        self.http_status = http_status


def _parse_error(resp: httpx.Response) -> OhdcRpcError:
    code = "unknown"
    message = resp.text or f"HTTP {resp.status_code}"
    try:
        body = resp.json()
        code = body.get("code", code)
        message = body.get("message", message)
    except Exception:
        pass
    return OhdcRpcError(code, message, resp.status_code)


class OhdcTransport:
    """Async Connect-RPC transport over HTTP/2 (h2c) or HTTP/1.1.

    Parameters:
        base_url: storage URL, e.g. ``"http://127.0.0.1:18443"``. Trailing
            slash optional.
        default_token: bearer token attached to every call when no
            per-call token is supplied. Used for Connect MCP's self-session
            mode; left ``None`` for Care + Emergency where each call
            carries its own grant token.
        timeout: per-call timeout in seconds. Default 30.
    """

    def __init__(
        self,
        *,
        base_url: str,
        default_token: str | None = None,
        timeout: float = 30.0,
    ) -> None:
        self._base_url = base_url.rstrip("/")
        self._default_token = default_token
        self._client = httpx.AsyncClient(timeout=timeout)

    async def aclose(self) -> None:
        await self._client.aclose()

    async def __aenter__(self) -> "OhdcTransport":
        return self

    async def __aexit__(self, *exc: object) -> None:
        await self.aclose()

    def _headers(self, token: str | None, *, streaming: bool) -> dict[str, str]:
        ct = "application/connect+proto" if streaming else "application/proto"
        h = {
            "Content-Type": ct,
            "Connect-Protocol-Version": "1",
        }
        bearer = token if token is not None else self._default_token
        if bearer:
            h["Authorization"] = f"Bearer {bearer}"
        return h

    async def call_unary(
        self,
        *,
        service: str,
        method: str,
        request: "Message",
        response_cls: type["Message"],
        token: str | None = None,
    ) -> "Message":
        """Issue a unary Connect-RPC call and parse the protobuf response."""
        url = f"{self._base_url}/{service}/{method}"
        body = request.SerializeToString()
        resp = await self._client.post(
            url,
            content=body,
            headers=self._headers(token, streaming=False),
        )
        if resp.status_code != 200:
            raise _parse_error(resp)
        out = response_cls()
        out.ParseFromString(resp.content)
        return out

    async def call_server_streaming(
        self,
        *,
        service: str,
        method: str,
        request: "Message",
        response_cls: type["Message"],
        token: str | None = None,
    ) -> AsyncIterator["Message"]:
        """Issue a server-streaming Connect-RPC call.

        Yields decoded response messages as they arrive. The final
        end-stream envelope's metadata is consumed but not surfaced.

        Per Connect protocol: each frame is ``flags(1) | len(4) | payload``.
        ``flags & 0x02`` marks the end-stream envelope (whose payload is
        JSON, not protobuf). Other flag bits are reserved.

        We frame the request body the same way (single envelope with
        ``flags=0``) per Connect's server-streaming spec.
        """
        url = f"{self._base_url}/{service}/{method}"
        req_body = request.SerializeToString()
        framed = b"\x00" + struct.pack(">I", len(req_body)) + req_body

        async with self._client.stream(
            "POST",
            url,
            content=framed,
            headers=self._headers(token, streaming=True),
        ) as resp:
            if resp.status_code != 200:
                # Drain so the connection can be reused, then surface the error.
                err_body = await resp.aread()
                try:
                    parsed = json.loads(err_body)
                    raise OhdcRpcError(
                        parsed.get("code", "unknown"),
                        parsed.get("message", err_body.decode("utf-8", "replace")),
                        resp.status_code,
                    )
                except (ValueError, AttributeError):
                    raise OhdcRpcError(
                        "unknown",
                        err_body.decode("utf-8", "replace") or f"HTTP {resp.status_code}",
                        resp.status_code,
                    ) from None

            buf = bytearray()
            async for chunk in resp.aiter_bytes():
                buf.extend(chunk)
                while True:
                    msg = self._take_frame(buf, response_cls)
                    if msg is _NOT_READY:
                        break
                    if msg is _END_STREAM:
                        return
                    yield msg

    @staticmethod
    def _take_frame(
        buf: bytearray, response_cls: type["Message"]
    ) -> "Message | _Sentinel":
        if len(buf) < 5:
            return _NOT_READY
        flags = buf[0]
        length = struct.unpack(">I", bytes(buf[1:5]))[0]
        if len(buf) < 5 + length:
            return _NOT_READY
        payload = bytes(buf[5 : 5 + length])
        del buf[: 5 + length]
        if flags & 0x02:
            # End-stream envelope: JSON. If it carries an error, raise.
            if payload:
                try:
                    parsed = json.loads(payload)
                except ValueError:
                    parsed = {}
                err = parsed.get("error")
                if err:
                    raise OhdcRpcError(
                        err.get("code", "unknown"),
                        err.get("message", "stream ended with error"),
                        200,
                    )
            return _END_STREAM
        out = response_cls()
        out.ParseFromString(payload)
        return out


class _Sentinel:
    """Sentinel for `_take_frame` results."""


_NOT_READY = _Sentinel()
_END_STREAM = _Sentinel()


__all__ = [
    "OhdcRpcError",
    "OhdcTransport",
]
