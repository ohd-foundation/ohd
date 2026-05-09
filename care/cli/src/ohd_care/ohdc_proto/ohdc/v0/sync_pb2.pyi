from ohdc.v0 import ohdc_pb2 as _ohdc_pb2
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class HelloRequest(_message.Message):
    __slots__ = ("peer_label", "peer_kind", "peer_ulid", "my_local_rowid_high_water", "my_inbound_watermark_for_you", "registry_version")
    PEER_LABEL_FIELD_NUMBER: _ClassVar[int]
    PEER_KIND_FIELD_NUMBER: _ClassVar[int]
    PEER_ULID_FIELD_NUMBER: _ClassVar[int]
    MY_LOCAL_ROWID_HIGH_WATER_FIELD_NUMBER: _ClassVar[int]
    MY_INBOUND_WATERMARK_FOR_YOU_FIELD_NUMBER: _ClassVar[int]
    REGISTRY_VERSION_FIELD_NUMBER: _ClassVar[int]
    peer_label: str
    peer_kind: str
    peer_ulid: _ohdc_pb2.Ulid
    my_local_rowid_high_water: int
    my_inbound_watermark_for_you: int
    registry_version: int
    def __init__(self, peer_label: _Optional[str] = ..., peer_kind: _Optional[str] = ..., peer_ulid: _Optional[_Union[_ohdc_pb2.Ulid, _Mapping]] = ..., my_local_rowid_high_water: _Optional[int] = ..., my_inbound_watermark_for_you: _Optional[int] = ..., registry_version: _Optional[int] = ...) -> None: ...

class HelloResponse(_message.Message):
    __slots__ = ("peer_label", "peer_kind", "peer_ulid", "my_local_rowid_high_water", "my_inbound_watermark_for_you", "registry_version", "caller_user_ulid")
    PEER_LABEL_FIELD_NUMBER: _ClassVar[int]
    PEER_KIND_FIELD_NUMBER: _ClassVar[int]
    PEER_ULID_FIELD_NUMBER: _ClassVar[int]
    MY_LOCAL_ROWID_HIGH_WATER_FIELD_NUMBER: _ClassVar[int]
    MY_INBOUND_WATERMARK_FOR_YOU_FIELD_NUMBER: _ClassVar[int]
    REGISTRY_VERSION_FIELD_NUMBER: _ClassVar[int]
    CALLER_USER_ULID_FIELD_NUMBER: _ClassVar[int]
    peer_label: str
    peer_kind: str
    peer_ulid: _ohdc_pb2.Ulid
    my_local_rowid_high_water: int
    my_inbound_watermark_for_you: int
    registry_version: int
    caller_user_ulid: _ohdc_pb2.Ulid
    def __init__(self, peer_label: _Optional[str] = ..., peer_kind: _Optional[str] = ..., peer_ulid: _Optional[_Union[_ohdc_pb2.Ulid, _Mapping]] = ..., my_local_rowid_high_water: _Optional[int] = ..., my_inbound_watermark_for_you: _Optional[int] = ..., registry_version: _Optional[int] = ..., caller_user_ulid: _Optional[_Union[_ohdc_pb2.Ulid, _Mapping]] = ...) -> None: ...

class PushFrame(_message.Message):
    __slots__ = ("sender_rowid", "event", "pending_event", "grant", "device", "app_version", "registry_entry")
    SENDER_ROWID_FIELD_NUMBER: _ClassVar[int]
    EVENT_FIELD_NUMBER: _ClassVar[int]
    PENDING_EVENT_FIELD_NUMBER: _ClassVar[int]
    GRANT_FIELD_NUMBER: _ClassVar[int]
    DEVICE_FIELD_NUMBER: _ClassVar[int]
    APP_VERSION_FIELD_NUMBER: _ClassVar[int]
    REGISTRY_ENTRY_FIELD_NUMBER: _ClassVar[int]
    sender_rowid: int
    event: EventFrame
    pending_event: PendingEventFrame
    grant: GrantFrame
    device: DeviceFrame
    app_version: AppVersionFrame
    registry_entry: RegistryEntryFrame
    def __init__(self, sender_rowid: _Optional[int] = ..., event: _Optional[_Union[EventFrame, _Mapping]] = ..., pending_event: _Optional[_Union[PendingEventFrame, _Mapping]] = ..., grant: _Optional[_Union[GrantFrame, _Mapping]] = ..., device: _Optional[_Union[DeviceFrame, _Mapping]] = ..., app_version: _Optional[_Union[AppVersionFrame, _Mapping]] = ..., registry_entry: _Optional[_Union[RegistryEntryFrame, _Mapping]] = ...) -> None: ...

class PushAck(_message.Message):
    __slots__ = ("sender_rowid", "outcome", "error")
    SENDER_ROWID_FIELD_NUMBER: _ClassVar[int]
    OUTCOME_FIELD_NUMBER: _ClassVar[int]
    ERROR_FIELD_NUMBER: _ClassVar[int]
    sender_rowid: int
    outcome: str
    error: _ohdc_pb2.ErrorInfo
    def __init__(self, sender_rowid: _Optional[int] = ..., outcome: _Optional[str] = ..., error: _Optional[_Union[_ohdc_pb2.ErrorInfo, _Mapping]] = ...) -> None: ...

class PullRequest(_message.Message):
    __slots__ = ("after_peer_rowid", "max_frames")
    AFTER_PEER_ROWID_FIELD_NUMBER: _ClassVar[int]
    MAX_FRAMES_FIELD_NUMBER: _ClassVar[int]
    after_peer_rowid: int
    max_frames: int
    def __init__(self, after_peer_rowid: _Optional[int] = ..., max_frames: _Optional[int] = ...) -> None: ...

class AttachmentAck(_message.Message):
    __slots__ = ("attachment_ulid", "sha256", "outcome")
    ATTACHMENT_ULID_FIELD_NUMBER: _ClassVar[int]
    SHA256_FIELD_NUMBER: _ClassVar[int]
    OUTCOME_FIELD_NUMBER: _ClassVar[int]
    attachment_ulid: _ohdc_pb2.Ulid
    sha256: bytes
    outcome: str
    def __init__(self, attachment_ulid: _Optional[_Union[_ohdc_pb2.Ulid, _Mapping]] = ..., sha256: _Optional[bytes] = ..., outcome: _Optional[str] = ...) -> None: ...

class PullAttachmentRequest(_message.Message):
    __slots__ = ("attachment_ulid",)
    ATTACHMENT_ULID_FIELD_NUMBER: _ClassVar[int]
    attachment_ulid: _ohdc_pb2.Ulid
    def __init__(self, attachment_ulid: _Optional[_Union[_ohdc_pb2.Ulid, _Mapping]] = ...) -> None: ...

class EventFrame(_message.Message):
    __slots__ = ("event",)
    EVENT_FIELD_NUMBER: _ClassVar[int]
    event: _ohdc_pb2.Event
    def __init__(self, event: _Optional[_Union[_ohdc_pb2.Event, _Mapping]] = ...) -> None: ...

class PendingEventFrame(_message.Message):
    __slots__ = ("pending_event",)
    PENDING_EVENT_FIELD_NUMBER: _ClassVar[int]
    pending_event: _ohdc_pb2.PendingEvent
    def __init__(self, pending_event: _Optional[_Union[_ohdc_pb2.PendingEvent, _Mapping]] = ...) -> None: ...

class GrantFrame(_message.Message):
    __slots__ = ("grant",)
    GRANT_FIELD_NUMBER: _ClassVar[int]
    grant: _ohdc_pb2.Grant
    def __init__(self, grant: _Optional[_Union[_ohdc_pb2.Grant, _Mapping]] = ...) -> None: ...

class DeviceFrame(_message.Message):
    __slots__ = ("kind", "vendor", "model", "serial_or_id", "metadata_json")
    KIND_FIELD_NUMBER: _ClassVar[int]
    VENDOR_FIELD_NUMBER: _ClassVar[int]
    MODEL_FIELD_NUMBER: _ClassVar[int]
    SERIAL_OR_ID_FIELD_NUMBER: _ClassVar[int]
    METADATA_JSON_FIELD_NUMBER: _ClassVar[int]
    kind: str
    vendor: str
    model: str
    serial_or_id: str
    metadata_json: str
    def __init__(self, kind: _Optional[str] = ..., vendor: _Optional[str] = ..., model: _Optional[str] = ..., serial_or_id: _Optional[str] = ..., metadata_json: _Optional[str] = ...) -> None: ...

class AppVersionFrame(_message.Message):
    __slots__ = ("app_name", "version", "platform")
    APP_NAME_FIELD_NUMBER: _ClassVar[int]
    VERSION_FIELD_NUMBER: _ClassVar[int]
    PLATFORM_FIELD_NUMBER: _ClassVar[int]
    app_name: str
    version: str
    platform: str
    def __init__(self, app_name: _Optional[str] = ..., version: _Optional[str] = ..., platform: _Optional[str] = ...) -> None: ...

class RegistryEntryFrame(_message.Message):
    __slots__ = ("kind", "payload")
    KIND_FIELD_NUMBER: _ClassVar[int]
    PAYLOAD_FIELD_NUMBER: _ClassVar[int]
    kind: str
    payload: bytes
    def __init__(self, kind: _Optional[str] = ..., payload: _Optional[bytes] = ...) -> None: ...
