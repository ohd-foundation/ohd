from ohdc.v0 import ohdc_pb2 as _ohdc_pb2
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from typing import ClassVar as _ClassVar, Optional as _Optional

DESCRIPTOR: _descriptor.FileDescriptor

class RegisterRequest(_message.Message):
    __slots__ = ("storage_cert_der", "label")
    STORAGE_CERT_DER_FIELD_NUMBER: _ClassVar[int]
    LABEL_FIELD_NUMBER: _ClassVar[int]
    storage_cert_der: bytes
    label: str
    def __init__(self, storage_cert_der: _Optional[bytes] = ..., label: _Optional[str] = ...) -> None: ...

class RegisterResponse(_message.Message):
    __slots__ = ("registration_token", "rendezvous_url", "pin_sha256")
    REGISTRATION_TOKEN_FIELD_NUMBER: _ClassVar[int]
    RENDEZVOUS_URL_FIELD_NUMBER: _ClassVar[int]
    PIN_SHA256_FIELD_NUMBER: _ClassVar[int]
    registration_token: str
    rendezvous_url: str
    pin_sha256: bytes
    def __init__(self, registration_token: _Optional[str] = ..., rendezvous_url: _Optional[str] = ..., pin_sha256: _Optional[bytes] = ...) -> None: ...

class RefreshRegistrationRequest(_message.Message):
    __slots__ = ("registration_token",)
    REGISTRATION_TOKEN_FIELD_NUMBER: _ClassVar[int]
    registration_token: str
    def __init__(self, registration_token: _Optional[str] = ...) -> None: ...

class RefreshRegistrationResponse(_message.Message):
    __slots__ = ("valid_until_ms",)
    VALID_UNTIL_MS_FIELD_NUMBER: _ClassVar[int]
    valid_until_ms: int
    def __init__(self, valid_until_ms: _Optional[int] = ...) -> None: ...

class HeartbeatRequest(_message.Message):
    __slots__ = ("registration_token", "client_time_ms")
    REGISTRATION_TOKEN_FIELD_NUMBER: _ClassVar[int]
    CLIENT_TIME_MS_FIELD_NUMBER: _ClassVar[int]
    registration_token: str
    client_time_ms: int
    def __init__(self, registration_token: _Optional[str] = ..., client_time_ms: _Optional[int] = ...) -> None: ...

class HeartbeatResponse(_message.Message):
    __slots__ = ("server_time_ms",)
    SERVER_TIME_MS_FIELD_NUMBER: _ClassVar[int]
    server_time_ms: int
    def __init__(self, server_time_ms: _Optional[int] = ...) -> None: ...

class DeregisterRequest(_message.Message):
    __slots__ = ("registration_token",)
    REGISTRATION_TOKEN_FIELD_NUMBER: _ClassVar[int]
    registration_token: str
    def __init__(self, registration_token: _Optional[str] = ...) -> None: ...

class DeregisterResponse(_message.Message):
    __slots__ = ("deregistered_at_ms",)
    DEREGISTERED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    deregistered_at_ms: int
    def __init__(self, deregistered_at_ms: _Optional[int] = ...) -> None: ...

class TunnelFrame(_message.Message):
    __slots__ = ("session_id", "payload", "kind")
    SESSION_ID_FIELD_NUMBER: _ClassVar[int]
    PAYLOAD_FIELD_NUMBER: _ClassVar[int]
    KIND_FIELD_NUMBER: _ClassVar[int]
    session_id: int
    payload: bytes
    kind: str
    def __init__(self, session_id: _Optional[int] = ..., payload: _Optional[bytes] = ..., kind: _Optional[str] = ...) -> None: ...
