import datetime

from google.protobuf import duration_pb2 as _duration_pb2
from google.protobuf import timestamp_pb2 as _timestamp_pb2
from google.protobuf import struct_pb2 as _struct_pb2
from google.protobuf.internal import containers as _containers
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class Sort(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    TIME_DESC: _ClassVar[Sort]
    TIME_ASC: _ClassVar[Sort]
    ULID_DESC: _ClassVar[Sort]
    ULID_ASC: _ClassVar[Sort]

class AggregateOp(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    AVG: _ClassVar[AggregateOp]
    SUM: _ClassVar[AggregateOp]
    MIN: _ClassVar[AggregateOp]
    MAX: _ClassVar[AggregateOp]
    COUNT: _ClassVar[AggregateOp]
    MEDIAN: _ClassVar[AggregateOp]
    P95: _ClassVar[AggregateOp]
    P99: _ClassVar[AggregateOp]
    STDDEV: _ClassVar[AggregateOp]

class CalendarUnit(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    HOUR: _ClassVar[CalendarUnit]
    DAY: _ClassVar[CalendarUnit]
    WEEK: _ClassVar[CalendarUnit]
    MONTH: _ClassVar[CalendarUnit]
    YEAR: _ClassVar[CalendarUnit]

class PendingQueryKind(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    PENDING_QUERY_KIND_UNSPECIFIED: _ClassVar[PendingQueryKind]
    QUERY_EVENTS: _ClassVar[PendingQueryKind]
    GET_EVENT_BY_ULID: _ClassVar[PendingQueryKind]
    AGGREGATE: _ClassVar[PendingQueryKind]
    CORRELATE: _ClassVar[PendingQueryKind]
    READ_SAMPLES: _ClassVar[PendingQueryKind]
    READ_ATTACHMENT: _ClassVar[PendingQueryKind]

class PendingQueryDecision(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    PENDING_QUERY_DECISION_UNSPECIFIED: _ClassVar[PendingQueryDecision]
    PENDING_QUERY_DECISION_PENDING: _ClassVar[PendingQueryDecision]
    PENDING_QUERY_DECISION_APPROVED: _ClassVar[PendingQueryDecision]
    PENDING_QUERY_DECISION_REJECTED: _ClassVar[PendingQueryDecision]
    PENDING_QUERY_DECISION_EXPIRED: _ClassVar[PendingQueryDecision]
TIME_DESC: Sort
TIME_ASC: Sort
ULID_DESC: Sort
ULID_ASC: Sort
AVG: AggregateOp
SUM: AggregateOp
MIN: AggregateOp
MAX: AggregateOp
COUNT: AggregateOp
MEDIAN: AggregateOp
P95: AggregateOp
P99: AggregateOp
STDDEV: AggregateOp
HOUR: CalendarUnit
DAY: CalendarUnit
WEEK: CalendarUnit
MONTH: CalendarUnit
YEAR: CalendarUnit
PENDING_QUERY_KIND_UNSPECIFIED: PendingQueryKind
QUERY_EVENTS: PendingQueryKind
GET_EVENT_BY_ULID: PendingQueryKind
AGGREGATE: PendingQueryKind
CORRELATE: PendingQueryKind
READ_SAMPLES: PendingQueryKind
READ_ATTACHMENT: PendingQueryKind
PENDING_QUERY_DECISION_UNSPECIFIED: PendingQueryDecision
PENDING_QUERY_DECISION_PENDING: PendingQueryDecision
PENDING_QUERY_DECISION_APPROVED: PendingQueryDecision
PENDING_QUERY_DECISION_REJECTED: PendingQueryDecision
PENDING_QUERY_DECISION_EXPIRED: PendingQueryDecision

class Ulid(_message.Message):
    __slots__ = ("bytes",)
    BYTES_FIELD_NUMBER: _ClassVar[int]
    bytes: bytes
    def __init__(self, bytes: _Optional[bytes] = ...) -> None: ...

class ChannelValue(_message.Message):
    __slots__ = ("channel_path", "real_value", "int_value", "bool_value", "text_value", "enum_ordinal")
    CHANNEL_PATH_FIELD_NUMBER: _ClassVar[int]
    REAL_VALUE_FIELD_NUMBER: _ClassVar[int]
    INT_VALUE_FIELD_NUMBER: _ClassVar[int]
    BOOL_VALUE_FIELD_NUMBER: _ClassVar[int]
    TEXT_VALUE_FIELD_NUMBER: _ClassVar[int]
    ENUM_ORDINAL_FIELD_NUMBER: _ClassVar[int]
    channel_path: str
    real_value: float
    int_value: int
    bool_value: bool
    text_value: str
    enum_ordinal: int
    def __init__(self, channel_path: _Optional[str] = ..., real_value: _Optional[float] = ..., int_value: _Optional[int] = ..., bool_value: bool = ..., text_value: _Optional[str] = ..., enum_ordinal: _Optional[int] = ...) -> None: ...

class SampleBlockRef(_message.Message):
    __slots__ = ("channel_path", "t0_ms", "t1_ms", "sample_count", "encoding")
    CHANNEL_PATH_FIELD_NUMBER: _ClassVar[int]
    T0_MS_FIELD_NUMBER: _ClassVar[int]
    T1_MS_FIELD_NUMBER: _ClassVar[int]
    SAMPLE_COUNT_FIELD_NUMBER: _ClassVar[int]
    ENCODING_FIELD_NUMBER: _ClassVar[int]
    channel_path: str
    t0_ms: int
    t1_ms: int
    sample_count: int
    encoding: int
    def __init__(self, channel_path: _Optional[str] = ..., t0_ms: _Optional[int] = ..., t1_ms: _Optional[int] = ..., sample_count: _Optional[int] = ..., encoding: _Optional[int] = ...) -> None: ...

class SampleBlockInput(_message.Message):
    __slots__ = ("channel_path", "t0_ms", "t1_ms", "sample_count", "encoding", "data")
    CHANNEL_PATH_FIELD_NUMBER: _ClassVar[int]
    T0_MS_FIELD_NUMBER: _ClassVar[int]
    T1_MS_FIELD_NUMBER: _ClassVar[int]
    SAMPLE_COUNT_FIELD_NUMBER: _ClassVar[int]
    ENCODING_FIELD_NUMBER: _ClassVar[int]
    DATA_FIELD_NUMBER: _ClassVar[int]
    channel_path: str
    t0_ms: int
    t1_ms: int
    sample_count: int
    encoding: int
    data: bytes
    def __init__(self, channel_path: _Optional[str] = ..., t0_ms: _Optional[int] = ..., t1_ms: _Optional[int] = ..., sample_count: _Optional[int] = ..., encoding: _Optional[int] = ..., data: _Optional[bytes] = ...) -> None: ...

class Sample(_message.Message):
    __slots__ = ("t_ms", "value")
    T_MS_FIELD_NUMBER: _ClassVar[int]
    VALUE_FIELD_NUMBER: _ClassVar[int]
    t_ms: int
    value: float
    def __init__(self, t_ms: _Optional[int] = ..., value: _Optional[float] = ...) -> None: ...

class AttachmentRef(_message.Message):
    __slots__ = ("ulid", "sha256", "byte_size", "mime_type", "filename")
    ULID_FIELD_NUMBER: _ClassVar[int]
    SHA256_FIELD_NUMBER: _ClassVar[int]
    BYTE_SIZE_FIELD_NUMBER: _ClassVar[int]
    MIME_TYPE_FIELD_NUMBER: _ClassVar[int]
    FILENAME_FIELD_NUMBER: _ClassVar[int]
    ulid: Ulid
    sha256: bytes
    byte_size: int
    mime_type: str
    filename: str
    def __init__(self, ulid: _Optional[_Union[Ulid, _Mapping]] = ..., sha256: _Optional[bytes] = ..., byte_size: _Optional[int] = ..., mime_type: _Optional[str] = ..., filename: _Optional[str] = ...) -> None: ...

class SourceSignature(_message.Message):
    __slots__ = ("sig_alg", "signer_kid", "signature")
    SIG_ALG_FIELD_NUMBER: _ClassVar[int]
    SIGNER_KID_FIELD_NUMBER: _ClassVar[int]
    SIGNATURE_FIELD_NUMBER: _ClassVar[int]
    sig_alg: str
    signer_kid: str
    signature: bytes
    def __init__(self, sig_alg: _Optional[str] = ..., signer_kid: _Optional[str] = ..., signature: _Optional[bytes] = ...) -> None: ...

class SignerInfo(_message.Message):
    __slots__ = ("signer_kid", "signer_label", "sig_alg", "revoked")
    SIGNER_KID_FIELD_NUMBER: _ClassVar[int]
    SIGNER_LABEL_FIELD_NUMBER: _ClassVar[int]
    SIG_ALG_FIELD_NUMBER: _ClassVar[int]
    REVOKED_FIELD_NUMBER: _ClassVar[int]
    signer_kid: str
    signer_label: str
    sig_alg: str
    revoked: bool
    def __init__(self, signer_kid: _Optional[str] = ..., signer_label: _Optional[str] = ..., sig_alg: _Optional[str] = ..., revoked: bool = ...) -> None: ...

class Metadata(_message.Message):
    __slots__ = ("entries",)
    class EntriesEntry(_message.Message):
        __slots__ = ("key", "value")
        KEY_FIELD_NUMBER: _ClassVar[int]
        VALUE_FIELD_NUMBER: _ClassVar[int]
        key: str
        value: str
        def __init__(self, key: _Optional[str] = ..., value: _Optional[str] = ...) -> None: ...
    ENTRIES_FIELD_NUMBER: _ClassVar[int]
    entries: _containers.ScalarMap[str, str]
    def __init__(self, entries: _Optional[_Mapping[str, str]] = ...) -> None: ...

class Event(_message.Message):
    __slots__ = ("ulid", "timestamp_ms", "duration_ms", "tz_offset_minutes", "tz_name", "event_type", "channels", "sample_blocks", "attachments", "device_id", "app_name", "app_version", "source", "source_id", "notes", "superseded_by", "deleted_at_ms", "metadata", "signed_by")
    ULID_FIELD_NUMBER: _ClassVar[int]
    TIMESTAMP_MS_FIELD_NUMBER: _ClassVar[int]
    DURATION_MS_FIELD_NUMBER: _ClassVar[int]
    TZ_OFFSET_MINUTES_FIELD_NUMBER: _ClassVar[int]
    TZ_NAME_FIELD_NUMBER: _ClassVar[int]
    EVENT_TYPE_FIELD_NUMBER: _ClassVar[int]
    CHANNELS_FIELD_NUMBER: _ClassVar[int]
    SAMPLE_BLOCKS_FIELD_NUMBER: _ClassVar[int]
    ATTACHMENTS_FIELD_NUMBER: _ClassVar[int]
    DEVICE_ID_FIELD_NUMBER: _ClassVar[int]
    APP_NAME_FIELD_NUMBER: _ClassVar[int]
    APP_VERSION_FIELD_NUMBER: _ClassVar[int]
    SOURCE_FIELD_NUMBER: _ClassVar[int]
    SOURCE_ID_FIELD_NUMBER: _ClassVar[int]
    NOTES_FIELD_NUMBER: _ClassVar[int]
    SUPERSEDED_BY_FIELD_NUMBER: _ClassVar[int]
    DELETED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    METADATA_FIELD_NUMBER: _ClassVar[int]
    SIGNED_BY_FIELD_NUMBER: _ClassVar[int]
    ulid: Ulid
    timestamp_ms: int
    duration_ms: int
    tz_offset_minutes: int
    tz_name: str
    event_type: str
    channels: _containers.RepeatedCompositeFieldContainer[ChannelValue]
    sample_blocks: _containers.RepeatedCompositeFieldContainer[SampleBlockRef]
    attachments: _containers.RepeatedCompositeFieldContainer[AttachmentRef]
    device_id: str
    app_name: str
    app_version: str
    source: str
    source_id: str
    notes: str
    superseded_by: Ulid
    deleted_at_ms: int
    metadata: Metadata
    signed_by: SignerInfo
    def __init__(self, ulid: _Optional[_Union[Ulid, _Mapping]] = ..., timestamp_ms: _Optional[int] = ..., duration_ms: _Optional[int] = ..., tz_offset_minutes: _Optional[int] = ..., tz_name: _Optional[str] = ..., event_type: _Optional[str] = ..., channels: _Optional[_Iterable[_Union[ChannelValue, _Mapping]]] = ..., sample_blocks: _Optional[_Iterable[_Union[SampleBlockRef, _Mapping]]] = ..., attachments: _Optional[_Iterable[_Union[AttachmentRef, _Mapping]]] = ..., device_id: _Optional[str] = ..., app_name: _Optional[str] = ..., app_version: _Optional[str] = ..., source: _Optional[str] = ..., source_id: _Optional[str] = ..., notes: _Optional[str] = ..., superseded_by: _Optional[_Union[Ulid, _Mapping]] = ..., deleted_at_ms: _Optional[int] = ..., metadata: _Optional[_Union[Metadata, _Mapping]] = ..., signed_by: _Optional[_Union[SignerInfo, _Mapping]] = ...) -> None: ...

class EventInput(_message.Message):
    __slots__ = ("timestamp_ms", "duration_ms", "tz_offset_minutes", "tz_name", "event_type", "channels", "sample_blocks", "attachment_ulids", "device_id", "app_name", "app_version", "source", "source_id", "notes", "superseded_by", "metadata", "source_signature")
    TIMESTAMP_MS_FIELD_NUMBER: _ClassVar[int]
    DURATION_MS_FIELD_NUMBER: _ClassVar[int]
    TZ_OFFSET_MINUTES_FIELD_NUMBER: _ClassVar[int]
    TZ_NAME_FIELD_NUMBER: _ClassVar[int]
    EVENT_TYPE_FIELD_NUMBER: _ClassVar[int]
    CHANNELS_FIELD_NUMBER: _ClassVar[int]
    SAMPLE_BLOCKS_FIELD_NUMBER: _ClassVar[int]
    ATTACHMENT_ULIDS_FIELD_NUMBER: _ClassVar[int]
    DEVICE_ID_FIELD_NUMBER: _ClassVar[int]
    APP_NAME_FIELD_NUMBER: _ClassVar[int]
    APP_VERSION_FIELD_NUMBER: _ClassVar[int]
    SOURCE_FIELD_NUMBER: _ClassVar[int]
    SOURCE_ID_FIELD_NUMBER: _ClassVar[int]
    NOTES_FIELD_NUMBER: _ClassVar[int]
    SUPERSEDED_BY_FIELD_NUMBER: _ClassVar[int]
    METADATA_FIELD_NUMBER: _ClassVar[int]
    SOURCE_SIGNATURE_FIELD_NUMBER: _ClassVar[int]
    timestamp_ms: int
    duration_ms: int
    tz_offset_minutes: int
    tz_name: str
    event_type: str
    channels: _containers.RepeatedCompositeFieldContainer[ChannelValue]
    sample_blocks: _containers.RepeatedCompositeFieldContainer[SampleBlockInput]
    attachment_ulids: _containers.RepeatedCompositeFieldContainer[Ulid]
    device_id: str
    app_name: str
    app_version: str
    source: str
    source_id: str
    notes: str
    superseded_by: Ulid
    metadata: Metadata
    source_signature: SourceSignature
    def __init__(self, timestamp_ms: _Optional[int] = ..., duration_ms: _Optional[int] = ..., tz_offset_minutes: _Optional[int] = ..., tz_name: _Optional[str] = ..., event_type: _Optional[str] = ..., channels: _Optional[_Iterable[_Union[ChannelValue, _Mapping]]] = ..., sample_blocks: _Optional[_Iterable[_Union[SampleBlockInput, _Mapping]]] = ..., attachment_ulids: _Optional[_Iterable[_Union[Ulid, _Mapping]]] = ..., device_id: _Optional[str] = ..., app_name: _Optional[str] = ..., app_version: _Optional[str] = ..., source: _Optional[str] = ..., source_id: _Optional[str] = ..., notes: _Optional[str] = ..., superseded_by: _Optional[_Union[Ulid, _Mapping]] = ..., metadata: _Optional[_Union[Metadata, _Mapping]] = ..., source_signature: _Optional[_Union[SourceSignature, _Mapping]] = ...) -> None: ...

class ErrorInfo(_message.Message):
    __slots__ = ("code", "message", "metadata")
    class MetadataEntry(_message.Message):
        __slots__ = ("key", "value")
        KEY_FIELD_NUMBER: _ClassVar[int]
        VALUE_FIELD_NUMBER: _ClassVar[int]
        key: str
        value: str
        def __init__(self, key: _Optional[str] = ..., value: _Optional[str] = ...) -> None: ...
    CODE_FIELD_NUMBER: _ClassVar[int]
    MESSAGE_FIELD_NUMBER: _ClassVar[int]
    METADATA_FIELD_NUMBER: _ClassVar[int]
    code: str
    message: str
    metadata: _containers.ScalarMap[str, str]
    def __init__(self, code: _Optional[str] = ..., message: _Optional[str] = ..., metadata: _Optional[_Mapping[str, str]] = ...) -> None: ...

class PageRequest(_message.Message):
    __slots__ = ("limit", "cursor")
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    CURSOR_FIELD_NUMBER: _ClassVar[int]
    limit: int
    cursor: str
    def __init__(self, limit: _Optional[int] = ..., cursor: _Optional[str] = ...) -> None: ...

class PageResponse(_message.Message):
    __slots__ = ("next_cursor",)
    NEXT_CURSOR_FIELD_NUMBER: _ClassVar[int]
    next_cursor: str
    def __init__(self, next_cursor: _Optional[str] = ...) -> None: ...

class EventFilter(_message.Message):
    __slots__ = ("from_ms", "to_ms", "event_types_in", "event_types_not_in", "channels", "source_in", "device_id_in", "sensitivity_classes_in", "event_ulids_in", "include_deleted", "include_superseded", "limit", "sort")
    FROM_MS_FIELD_NUMBER: _ClassVar[int]
    TO_MS_FIELD_NUMBER: _ClassVar[int]
    EVENT_TYPES_IN_FIELD_NUMBER: _ClassVar[int]
    EVENT_TYPES_NOT_IN_FIELD_NUMBER: _ClassVar[int]
    CHANNELS_FIELD_NUMBER: _ClassVar[int]
    SOURCE_IN_FIELD_NUMBER: _ClassVar[int]
    DEVICE_ID_IN_FIELD_NUMBER: _ClassVar[int]
    SENSITIVITY_CLASSES_IN_FIELD_NUMBER: _ClassVar[int]
    EVENT_ULIDS_IN_FIELD_NUMBER: _ClassVar[int]
    INCLUDE_DELETED_FIELD_NUMBER: _ClassVar[int]
    INCLUDE_SUPERSEDED_FIELD_NUMBER: _ClassVar[int]
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    SORT_FIELD_NUMBER: _ClassVar[int]
    from_ms: int
    to_ms: int
    event_types_in: _containers.RepeatedScalarFieldContainer[str]
    event_types_not_in: _containers.RepeatedScalarFieldContainer[str]
    channels: _containers.RepeatedCompositeFieldContainer[ChannelPredicate]
    source_in: _containers.RepeatedScalarFieldContainer[str]
    device_id_in: _containers.RepeatedScalarFieldContainer[str]
    sensitivity_classes_in: _containers.RepeatedScalarFieldContainer[str]
    event_ulids_in: _containers.RepeatedCompositeFieldContainer[Ulid]
    include_deleted: bool
    include_superseded: bool
    limit: int
    sort: Sort
    def __init__(self, from_ms: _Optional[int] = ..., to_ms: _Optional[int] = ..., event_types_in: _Optional[_Iterable[str]] = ..., event_types_not_in: _Optional[_Iterable[str]] = ..., channels: _Optional[_Iterable[_Union[ChannelPredicate, _Mapping]]] = ..., source_in: _Optional[_Iterable[str]] = ..., device_id_in: _Optional[_Iterable[str]] = ..., sensitivity_classes_in: _Optional[_Iterable[str]] = ..., event_ulids_in: _Optional[_Iterable[_Union[Ulid, _Mapping]]] = ..., include_deleted: bool = ..., include_superseded: bool = ..., limit: _Optional[int] = ..., sort: _Optional[_Union[Sort, str]] = ...) -> None: ...

class ChannelPredicate(_message.Message):
    __slots__ = ("channel_path", "real_range", "int_range", "exists", "enum_in", "text_contains")
    CHANNEL_PATH_FIELD_NUMBER: _ClassVar[int]
    REAL_RANGE_FIELD_NUMBER: _ClassVar[int]
    INT_RANGE_FIELD_NUMBER: _ClassVar[int]
    EXISTS_FIELD_NUMBER: _ClassVar[int]
    ENUM_IN_FIELD_NUMBER: _ClassVar[int]
    TEXT_CONTAINS_FIELD_NUMBER: _ClassVar[int]
    channel_path: str
    real_range: Range
    int_range: Range
    exists: bool
    enum_in: EnumIn
    text_contains: str
    def __init__(self, channel_path: _Optional[str] = ..., real_range: _Optional[_Union[Range, _Mapping]] = ..., int_range: _Optional[_Union[Range, _Mapping]] = ..., exists: bool = ..., enum_in: _Optional[_Union[EnumIn, _Mapping]] = ..., text_contains: _Optional[str] = ...) -> None: ...

class Range(_message.Message):
    __slots__ = ("min", "max", "min_inclusive", "max_inclusive")
    MIN_FIELD_NUMBER: _ClassVar[int]
    MAX_FIELD_NUMBER: _ClassVar[int]
    MIN_INCLUSIVE_FIELD_NUMBER: _ClassVar[int]
    MAX_INCLUSIVE_FIELD_NUMBER: _ClassVar[int]
    min: float
    max: float
    min_inclusive: bool
    max_inclusive: bool
    def __init__(self, min: _Optional[float] = ..., max: _Optional[float] = ..., min_inclusive: bool = ..., max_inclusive: bool = ...) -> None: ...

class EnumIn(_message.Message):
    __slots__ = ("ordinals",)
    ORDINALS_FIELD_NUMBER: _ClassVar[int]
    ordinals: _containers.RepeatedScalarFieldContainer[int]
    def __init__(self, ordinals: _Optional[_Iterable[int]] = ...) -> None: ...

class PutEventsRequest(_message.Message):
    __slots__ = ("events", "atomic")
    EVENTS_FIELD_NUMBER: _ClassVar[int]
    ATOMIC_FIELD_NUMBER: _ClassVar[int]
    events: _containers.RepeatedCompositeFieldContainer[EventInput]
    atomic: bool
    def __init__(self, events: _Optional[_Iterable[_Union[EventInput, _Mapping]]] = ..., atomic: bool = ...) -> None: ...

class PutEventsResponse(_message.Message):
    __slots__ = ("results",)
    RESULTS_FIELD_NUMBER: _ClassVar[int]
    results: _containers.RepeatedCompositeFieldContainer[PutEventResult]
    def __init__(self, results: _Optional[_Iterable[_Union[PutEventResult, _Mapping]]] = ...) -> None: ...

class PutEventResult(_message.Message):
    __slots__ = ("committed", "pending", "error")
    COMMITTED_FIELD_NUMBER: _ClassVar[int]
    PENDING_FIELD_NUMBER: _ClassVar[int]
    ERROR_FIELD_NUMBER: _ClassVar[int]
    committed: PutEventCommitted
    pending: PutEventPending
    error: ErrorInfo
    def __init__(self, committed: _Optional[_Union[PutEventCommitted, _Mapping]] = ..., pending: _Optional[_Union[PutEventPending, _Mapping]] = ..., error: _Optional[_Union[ErrorInfo, _Mapping]] = ...) -> None: ...

class PutEventCommitted(_message.Message):
    __slots__ = ("ulid", "committed_at_ms")
    ULID_FIELD_NUMBER: _ClassVar[int]
    COMMITTED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    ulid: Ulid
    committed_at_ms: int
    def __init__(self, ulid: _Optional[_Union[Ulid, _Mapping]] = ..., committed_at_ms: _Optional[int] = ...) -> None: ...

class PutEventPending(_message.Message):
    __slots__ = ("ulid", "expires_at_ms")
    ULID_FIELD_NUMBER: _ClassVar[int]
    EXPIRES_AT_MS_FIELD_NUMBER: _ClassVar[int]
    ulid: Ulid
    expires_at_ms: int
    def __init__(self, ulid: _Optional[_Union[Ulid, _Mapping]] = ..., expires_at_ms: _Optional[int] = ...) -> None: ...

class AttachBlobChunk(_message.Message):
    __slots__ = ("init", "data", "finish")
    INIT_FIELD_NUMBER: _ClassVar[int]
    DATA_FIELD_NUMBER: _ClassVar[int]
    FINISH_FIELD_NUMBER: _ClassVar[int]
    init: AttachBlobInit
    data: bytes
    finish: AttachBlobFinish
    def __init__(self, init: _Optional[_Union[AttachBlobInit, _Mapping]] = ..., data: _Optional[bytes] = ..., finish: _Optional[_Union[AttachBlobFinish, _Mapping]] = ...) -> None: ...

class AttachBlobInit(_message.Message):
    __slots__ = ("mime_type", "filename", "ulid", "expected_byte_size")
    MIME_TYPE_FIELD_NUMBER: _ClassVar[int]
    FILENAME_FIELD_NUMBER: _ClassVar[int]
    ULID_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_BYTE_SIZE_FIELD_NUMBER: _ClassVar[int]
    mime_type: str
    filename: str
    ulid: Ulid
    expected_byte_size: int
    def __init__(self, mime_type: _Optional[str] = ..., filename: _Optional[str] = ..., ulid: _Optional[_Union[Ulid, _Mapping]] = ..., expected_byte_size: _Optional[int] = ...) -> None: ...

class AttachBlobFinish(_message.Message):
    __slots__ = ("expected_sha256",)
    EXPECTED_SHA256_FIELD_NUMBER: _ClassVar[int]
    expected_sha256: bytes
    def __init__(self, expected_sha256: _Optional[bytes] = ...) -> None: ...

class AttachBlobResponse(_message.Message):
    __slots__ = ("attachment",)
    ATTACHMENT_FIELD_NUMBER: _ClassVar[int]
    attachment: AttachmentRef
    def __init__(self, attachment: _Optional[_Union[AttachmentRef, _Mapping]] = ...) -> None: ...

class QueryEventsRequest(_message.Message):
    __slots__ = ("filter",)
    FILTER_FIELD_NUMBER: _ClassVar[int]
    filter: EventFilter
    def __init__(self, filter: _Optional[_Union[EventFilter, _Mapping]] = ...) -> None: ...

class GetEventByUlidRequest(_message.Message):
    __slots__ = ("ulid",)
    ULID_FIELD_NUMBER: _ClassVar[int]
    ulid: Ulid
    def __init__(self, ulid: _Optional[_Union[Ulid, _Mapping]] = ...) -> None: ...

class AggregateRequest(_message.Message):
    __slots__ = ("channel_path", "filter", "op", "bucket")
    CHANNEL_PATH_FIELD_NUMBER: _ClassVar[int]
    FILTER_FIELD_NUMBER: _ClassVar[int]
    OP_FIELD_NUMBER: _ClassVar[int]
    BUCKET_FIELD_NUMBER: _ClassVar[int]
    channel_path: str
    filter: EventFilter
    op: AggregateOp
    bucket: Bucket
    def __init__(self, channel_path: _Optional[str] = ..., filter: _Optional[_Union[EventFilter, _Mapping]] = ..., op: _Optional[_Union[AggregateOp, str]] = ..., bucket: _Optional[_Union[Bucket, _Mapping]] = ...) -> None: ...

class Bucket(_message.Message):
    __slots__ = ("fixed", "calendar")
    FIXED_FIELD_NUMBER: _ClassVar[int]
    CALENDAR_FIELD_NUMBER: _ClassVar[int]
    fixed: _duration_pb2.Duration
    calendar: CalendarBucket
    def __init__(self, fixed: _Optional[_Union[datetime.timedelta, _duration_pb2.Duration, _Mapping]] = ..., calendar: _Optional[_Union[CalendarBucket, _Mapping]] = ...) -> None: ...

class CalendarBucket(_message.Message):
    __slots__ = ("tz_name", "unit")
    TZ_NAME_FIELD_NUMBER: _ClassVar[int]
    UNIT_FIELD_NUMBER: _ClassVar[int]
    tz_name: str
    unit: CalendarUnit
    def __init__(self, tz_name: _Optional[str] = ..., unit: _Optional[_Union[CalendarUnit, str]] = ...) -> None: ...

class AggregateResponse(_message.Message):
    __slots__ = ("buckets",)
    BUCKETS_FIELD_NUMBER: _ClassVar[int]
    buckets: _containers.RepeatedCompositeFieldContainer[AggregateBucketResult]
    def __init__(self, buckets: _Optional[_Iterable[_Union[AggregateBucketResult, _Mapping]]] = ...) -> None: ...

class AggregateBucketResult(_message.Message):
    __slots__ = ("bucket_start_ms", "bucket_end_ms", "sample_count", "value")
    BUCKET_START_MS_FIELD_NUMBER: _ClassVar[int]
    BUCKET_END_MS_FIELD_NUMBER: _ClassVar[int]
    SAMPLE_COUNT_FIELD_NUMBER: _ClassVar[int]
    VALUE_FIELD_NUMBER: _ClassVar[int]
    bucket_start_ms: int
    bucket_end_ms: int
    sample_count: int
    value: float
    def __init__(self, bucket_start_ms: _Optional[int] = ..., bucket_end_ms: _Optional[int] = ..., sample_count: _Optional[int] = ..., value: _Optional[float] = ...) -> None: ...

class CorrelateRequest(_message.Message):
    __slots__ = ("a", "b", "window", "scope")
    A_FIELD_NUMBER: _ClassVar[int]
    B_FIELD_NUMBER: _ClassVar[int]
    WINDOW_FIELD_NUMBER: _ClassVar[int]
    SCOPE_FIELD_NUMBER: _ClassVar[int]
    a: CorrelateSide
    b: CorrelateSide
    window: _duration_pb2.Duration
    scope: EventFilter
    def __init__(self, a: _Optional[_Union[CorrelateSide, _Mapping]] = ..., b: _Optional[_Union[CorrelateSide, _Mapping]] = ..., window: _Optional[_Union[datetime.timedelta, _duration_pb2.Duration, _Mapping]] = ..., scope: _Optional[_Union[EventFilter, _Mapping]] = ...) -> None: ...

class CorrelateSide(_message.Message):
    __slots__ = ("event_type", "channel_path")
    EVENT_TYPE_FIELD_NUMBER: _ClassVar[int]
    CHANNEL_PATH_FIELD_NUMBER: _ClassVar[int]
    event_type: str
    channel_path: str
    def __init__(self, event_type: _Optional[str] = ..., channel_path: _Optional[str] = ...) -> None: ...

class CorrelateResponse(_message.Message):
    __slots__ = ("pairs", "stats")
    PAIRS_FIELD_NUMBER: _ClassVar[int]
    STATS_FIELD_NUMBER: _ClassVar[int]
    pairs: _containers.RepeatedCompositeFieldContainer[CorrelatePair]
    stats: CorrelateStats
    def __init__(self, pairs: _Optional[_Iterable[_Union[CorrelatePair, _Mapping]]] = ..., stats: _Optional[_Union[CorrelateStats, _Mapping]] = ...) -> None: ...

class CorrelatePair(_message.Message):
    __slots__ = ("a_ulid", "a_time_ms", "matches")
    A_ULID_FIELD_NUMBER: _ClassVar[int]
    A_TIME_MS_FIELD_NUMBER: _ClassVar[int]
    MATCHES_FIELD_NUMBER: _ClassVar[int]
    a_ulid: Ulid
    a_time_ms: int
    matches: _containers.RepeatedCompositeFieldContainer[CorrelateMatch]
    def __init__(self, a_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., a_time_ms: _Optional[int] = ..., matches: _Optional[_Iterable[_Union[CorrelateMatch, _Mapping]]] = ...) -> None: ...

class CorrelateMatch(_message.Message):
    __slots__ = ("b_ulid", "b_time_ms", "b_value")
    B_ULID_FIELD_NUMBER: _ClassVar[int]
    B_TIME_MS_FIELD_NUMBER: _ClassVar[int]
    B_VALUE_FIELD_NUMBER: _ClassVar[int]
    b_ulid: Ulid
    b_time_ms: int
    b_value: float
    def __init__(self, b_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., b_time_ms: _Optional[int] = ..., b_value: _Optional[float] = ...) -> None: ...

class CorrelateStats(_message.Message):
    __slots__ = ("a_count", "b_count", "paired_count", "mean_b_value", "mean_lag_ms")
    A_COUNT_FIELD_NUMBER: _ClassVar[int]
    B_COUNT_FIELD_NUMBER: _ClassVar[int]
    PAIRED_COUNT_FIELD_NUMBER: _ClassVar[int]
    MEAN_B_VALUE_FIELD_NUMBER: _ClassVar[int]
    MEAN_LAG_MS_FIELD_NUMBER: _ClassVar[int]
    a_count: int
    b_count: int
    paired_count: int
    mean_b_value: float
    mean_lag_ms: float
    def __init__(self, a_count: _Optional[int] = ..., b_count: _Optional[int] = ..., paired_count: _Optional[int] = ..., mean_b_value: _Optional[float] = ..., mean_lag_ms: _Optional[float] = ...) -> None: ...

class ReadSamplesRequest(_message.Message):
    __slots__ = ("event_ulid", "channel_path", "from_ms", "to_ms", "max_samples")
    EVENT_ULID_FIELD_NUMBER: _ClassVar[int]
    CHANNEL_PATH_FIELD_NUMBER: _ClassVar[int]
    FROM_MS_FIELD_NUMBER: _ClassVar[int]
    TO_MS_FIELD_NUMBER: _ClassVar[int]
    MAX_SAMPLES_FIELD_NUMBER: _ClassVar[int]
    event_ulid: Ulid
    channel_path: str
    from_ms: int
    to_ms: int
    max_samples: int
    def __init__(self, event_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., channel_path: _Optional[str] = ..., from_ms: _Optional[int] = ..., to_ms: _Optional[int] = ..., max_samples: _Optional[int] = ...) -> None: ...

class SampleBatch(_message.Message):
    __slots__ = ("samples",)
    SAMPLES_FIELD_NUMBER: _ClassVar[int]
    samples: _containers.RepeatedCompositeFieldContainer[Sample]
    def __init__(self, samples: _Optional[_Iterable[_Union[Sample, _Mapping]]] = ...) -> None: ...

class ReadAttachmentRequest(_message.Message):
    __slots__ = ("attachment_ulid",)
    ATTACHMENT_ULID_FIELD_NUMBER: _ClassVar[int]
    attachment_ulid: Ulid
    def __init__(self, attachment_ulid: _Optional[_Union[Ulid, _Mapping]] = ...) -> None: ...

class AttachmentChunk(_message.Message):
    __slots__ = ("init", "data", "finish")
    INIT_FIELD_NUMBER: _ClassVar[int]
    DATA_FIELD_NUMBER: _ClassVar[int]
    FINISH_FIELD_NUMBER: _ClassVar[int]
    init: AttachmentInit
    data: bytes
    finish: AttachmentFinish
    def __init__(self, init: _Optional[_Union[AttachmentInit, _Mapping]] = ..., data: _Optional[bytes] = ..., finish: _Optional[_Union[AttachmentFinish, _Mapping]] = ...) -> None: ...

class AttachmentInit(_message.Message):
    __slots__ = ("ref",)
    REF_FIELD_NUMBER: _ClassVar[int]
    ref: AttachmentRef
    def __init__(self, ref: _Optional[_Union[AttachmentRef, _Mapping]] = ...) -> None: ...

class AttachmentFinish(_message.Message):
    __slots__ = ("expected_sha256",)
    EXPECTED_SHA256_FIELD_NUMBER: _ClassVar[int]
    expected_sha256: bytes
    def __init__(self, expected_sha256: _Optional[bytes] = ...) -> None: ...

class Grant(_message.Message):
    __slots__ = ("ulid", "grantee_label", "grantee_kind", "grantee_ulid", "purpose", "created_at_ms", "expires_at_ms", "revoked_at_ms", "default_action", "aggregation_only", "strip_notes", "require_approval_per_query", "rolling_window_days", "absolute_window", "event_type_rules", "channel_rules", "sensitivity_rules", "approval_mode", "write_event_type_rules", "auto_approve_event_types", "notify_on_access", "max_queries_per_day", "max_queries_per_hour", "case_ulids", "last_used_ms", "use_count")
    ULID_FIELD_NUMBER: _ClassVar[int]
    GRANTEE_LABEL_FIELD_NUMBER: _ClassVar[int]
    GRANTEE_KIND_FIELD_NUMBER: _ClassVar[int]
    GRANTEE_ULID_FIELD_NUMBER: _ClassVar[int]
    PURPOSE_FIELD_NUMBER: _ClassVar[int]
    CREATED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    EXPIRES_AT_MS_FIELD_NUMBER: _ClassVar[int]
    REVOKED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    DEFAULT_ACTION_FIELD_NUMBER: _ClassVar[int]
    AGGREGATION_ONLY_FIELD_NUMBER: _ClassVar[int]
    STRIP_NOTES_FIELD_NUMBER: _ClassVar[int]
    REQUIRE_APPROVAL_PER_QUERY_FIELD_NUMBER: _ClassVar[int]
    ROLLING_WINDOW_DAYS_FIELD_NUMBER: _ClassVar[int]
    ABSOLUTE_WINDOW_FIELD_NUMBER: _ClassVar[int]
    EVENT_TYPE_RULES_FIELD_NUMBER: _ClassVar[int]
    CHANNEL_RULES_FIELD_NUMBER: _ClassVar[int]
    SENSITIVITY_RULES_FIELD_NUMBER: _ClassVar[int]
    APPROVAL_MODE_FIELD_NUMBER: _ClassVar[int]
    WRITE_EVENT_TYPE_RULES_FIELD_NUMBER: _ClassVar[int]
    AUTO_APPROVE_EVENT_TYPES_FIELD_NUMBER: _ClassVar[int]
    NOTIFY_ON_ACCESS_FIELD_NUMBER: _ClassVar[int]
    MAX_QUERIES_PER_DAY_FIELD_NUMBER: _ClassVar[int]
    MAX_QUERIES_PER_HOUR_FIELD_NUMBER: _ClassVar[int]
    CASE_ULIDS_FIELD_NUMBER: _ClassVar[int]
    LAST_USED_MS_FIELD_NUMBER: _ClassVar[int]
    USE_COUNT_FIELD_NUMBER: _ClassVar[int]
    ulid: Ulid
    grantee_label: str
    grantee_kind: str
    grantee_ulid: Ulid
    purpose: str
    created_at_ms: int
    expires_at_ms: int
    revoked_at_ms: int
    default_action: str
    aggregation_only: bool
    strip_notes: bool
    require_approval_per_query: bool
    rolling_window_days: int
    absolute_window: TimeWindow
    event_type_rules: _containers.RepeatedCompositeFieldContainer[GrantEventTypeRule]
    channel_rules: _containers.RepeatedCompositeFieldContainer[GrantChannelRule]
    sensitivity_rules: _containers.RepeatedCompositeFieldContainer[GrantSensitivityRule]
    approval_mode: str
    write_event_type_rules: _containers.RepeatedCompositeFieldContainer[GrantWriteEventTypeRule]
    auto_approve_event_types: _containers.RepeatedScalarFieldContainer[str]
    notify_on_access: bool
    max_queries_per_day: int
    max_queries_per_hour: int
    case_ulids: _containers.RepeatedCompositeFieldContainer[Ulid]
    last_used_ms: int
    use_count: int
    def __init__(self, ulid: _Optional[_Union[Ulid, _Mapping]] = ..., grantee_label: _Optional[str] = ..., grantee_kind: _Optional[str] = ..., grantee_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., purpose: _Optional[str] = ..., created_at_ms: _Optional[int] = ..., expires_at_ms: _Optional[int] = ..., revoked_at_ms: _Optional[int] = ..., default_action: _Optional[str] = ..., aggregation_only: bool = ..., strip_notes: bool = ..., require_approval_per_query: bool = ..., rolling_window_days: _Optional[int] = ..., absolute_window: _Optional[_Union[TimeWindow, _Mapping]] = ..., event_type_rules: _Optional[_Iterable[_Union[GrantEventTypeRule, _Mapping]]] = ..., channel_rules: _Optional[_Iterable[_Union[GrantChannelRule, _Mapping]]] = ..., sensitivity_rules: _Optional[_Iterable[_Union[GrantSensitivityRule, _Mapping]]] = ..., approval_mode: _Optional[str] = ..., write_event_type_rules: _Optional[_Iterable[_Union[GrantWriteEventTypeRule, _Mapping]]] = ..., auto_approve_event_types: _Optional[_Iterable[str]] = ..., notify_on_access: bool = ..., max_queries_per_day: _Optional[int] = ..., max_queries_per_hour: _Optional[int] = ..., case_ulids: _Optional[_Iterable[_Union[Ulid, _Mapping]]] = ..., last_used_ms: _Optional[int] = ..., use_count: _Optional[int] = ...) -> None: ...

class TimeWindow(_message.Message):
    __slots__ = ("from_ms", "to_ms")
    FROM_MS_FIELD_NUMBER: _ClassVar[int]
    TO_MS_FIELD_NUMBER: _ClassVar[int]
    from_ms: int
    to_ms: int
    def __init__(self, from_ms: _Optional[int] = ..., to_ms: _Optional[int] = ...) -> None: ...

class GrantEventTypeRule(_message.Message):
    __slots__ = ("event_type", "effect")
    EVENT_TYPE_FIELD_NUMBER: _ClassVar[int]
    EFFECT_FIELD_NUMBER: _ClassVar[int]
    event_type: str
    effect: str
    def __init__(self, event_type: _Optional[str] = ..., effect: _Optional[str] = ...) -> None: ...

class GrantChannelRule(_message.Message):
    __slots__ = ("channel_path", "effect")
    CHANNEL_PATH_FIELD_NUMBER: _ClassVar[int]
    EFFECT_FIELD_NUMBER: _ClassVar[int]
    channel_path: str
    effect: str
    def __init__(self, channel_path: _Optional[str] = ..., effect: _Optional[str] = ...) -> None: ...

class GrantSensitivityRule(_message.Message):
    __slots__ = ("sensitivity_class", "effect")
    SENSITIVITY_CLASS_FIELD_NUMBER: _ClassVar[int]
    EFFECT_FIELD_NUMBER: _ClassVar[int]
    sensitivity_class: str
    effect: str
    def __init__(self, sensitivity_class: _Optional[str] = ..., effect: _Optional[str] = ...) -> None: ...

class GrantWriteEventTypeRule(_message.Message):
    __slots__ = ("event_type", "effect")
    EVENT_TYPE_FIELD_NUMBER: _ClassVar[int]
    EFFECT_FIELD_NUMBER: _ClassVar[int]
    event_type: str
    effect: str
    def __init__(self, event_type: _Optional[str] = ..., effect: _Optional[str] = ...) -> None: ...

class CreateGrantRequest(_message.Message):
    __slots__ = ("grantee_label", "grantee_kind", "grantee_ulid", "purpose", "default_action", "aggregation_only", "strip_notes", "require_approval_per_query", "rolling_window_days", "absolute_window", "event_type_rules", "channel_rules", "sensitivity_rules", "approval_mode", "write_event_type_rules", "auto_approve_event_types", "expires_at_ms", "notify_on_access", "max_queries_per_day", "max_queries_per_hour", "case_ulids")
    GRANTEE_LABEL_FIELD_NUMBER: _ClassVar[int]
    GRANTEE_KIND_FIELD_NUMBER: _ClassVar[int]
    GRANTEE_ULID_FIELD_NUMBER: _ClassVar[int]
    PURPOSE_FIELD_NUMBER: _ClassVar[int]
    DEFAULT_ACTION_FIELD_NUMBER: _ClassVar[int]
    AGGREGATION_ONLY_FIELD_NUMBER: _ClassVar[int]
    STRIP_NOTES_FIELD_NUMBER: _ClassVar[int]
    REQUIRE_APPROVAL_PER_QUERY_FIELD_NUMBER: _ClassVar[int]
    ROLLING_WINDOW_DAYS_FIELD_NUMBER: _ClassVar[int]
    ABSOLUTE_WINDOW_FIELD_NUMBER: _ClassVar[int]
    EVENT_TYPE_RULES_FIELD_NUMBER: _ClassVar[int]
    CHANNEL_RULES_FIELD_NUMBER: _ClassVar[int]
    SENSITIVITY_RULES_FIELD_NUMBER: _ClassVar[int]
    APPROVAL_MODE_FIELD_NUMBER: _ClassVar[int]
    WRITE_EVENT_TYPE_RULES_FIELD_NUMBER: _ClassVar[int]
    AUTO_APPROVE_EVENT_TYPES_FIELD_NUMBER: _ClassVar[int]
    EXPIRES_AT_MS_FIELD_NUMBER: _ClassVar[int]
    NOTIFY_ON_ACCESS_FIELD_NUMBER: _ClassVar[int]
    MAX_QUERIES_PER_DAY_FIELD_NUMBER: _ClassVar[int]
    MAX_QUERIES_PER_HOUR_FIELD_NUMBER: _ClassVar[int]
    CASE_ULIDS_FIELD_NUMBER: _ClassVar[int]
    grantee_label: str
    grantee_kind: str
    grantee_ulid: Ulid
    purpose: str
    default_action: str
    aggregation_only: bool
    strip_notes: bool
    require_approval_per_query: bool
    rolling_window_days: int
    absolute_window: TimeWindow
    event_type_rules: _containers.RepeatedCompositeFieldContainer[GrantEventTypeRule]
    channel_rules: _containers.RepeatedCompositeFieldContainer[GrantChannelRule]
    sensitivity_rules: _containers.RepeatedCompositeFieldContainer[GrantSensitivityRule]
    approval_mode: str
    write_event_type_rules: _containers.RepeatedCompositeFieldContainer[GrantWriteEventTypeRule]
    auto_approve_event_types: _containers.RepeatedScalarFieldContainer[str]
    expires_at_ms: int
    notify_on_access: bool
    max_queries_per_day: int
    max_queries_per_hour: int
    case_ulids: _containers.RepeatedCompositeFieldContainer[Ulid]
    def __init__(self, grantee_label: _Optional[str] = ..., grantee_kind: _Optional[str] = ..., grantee_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., purpose: _Optional[str] = ..., default_action: _Optional[str] = ..., aggregation_only: bool = ..., strip_notes: bool = ..., require_approval_per_query: bool = ..., rolling_window_days: _Optional[int] = ..., absolute_window: _Optional[_Union[TimeWindow, _Mapping]] = ..., event_type_rules: _Optional[_Iterable[_Union[GrantEventTypeRule, _Mapping]]] = ..., channel_rules: _Optional[_Iterable[_Union[GrantChannelRule, _Mapping]]] = ..., sensitivity_rules: _Optional[_Iterable[_Union[GrantSensitivityRule, _Mapping]]] = ..., approval_mode: _Optional[str] = ..., write_event_type_rules: _Optional[_Iterable[_Union[GrantWriteEventTypeRule, _Mapping]]] = ..., auto_approve_event_types: _Optional[_Iterable[str]] = ..., expires_at_ms: _Optional[int] = ..., notify_on_access: bool = ..., max_queries_per_day: _Optional[int] = ..., max_queries_per_hour: _Optional[int] = ..., case_ulids: _Optional[_Iterable[_Union[Ulid, _Mapping]]] = ...) -> None: ...

class CreateGrantResponse(_message.Message):
    __slots__ = ("grant", "token", "share_url", "share_qr_png")
    GRANT_FIELD_NUMBER: _ClassVar[int]
    TOKEN_FIELD_NUMBER: _ClassVar[int]
    SHARE_URL_FIELD_NUMBER: _ClassVar[int]
    SHARE_QR_PNG_FIELD_NUMBER: _ClassVar[int]
    grant: Grant
    token: str
    share_url: str
    share_qr_png: bytes
    def __init__(self, grant: _Optional[_Union[Grant, _Mapping]] = ..., token: _Optional[str] = ..., share_url: _Optional[str] = ..., share_qr_png: _Optional[bytes] = ...) -> None: ...

class ListGrantsRequest(_message.Message):
    __slots__ = ("include_revoked", "include_expired", "grantee_kind", "page")
    INCLUDE_REVOKED_FIELD_NUMBER: _ClassVar[int]
    INCLUDE_EXPIRED_FIELD_NUMBER: _ClassVar[int]
    GRANTEE_KIND_FIELD_NUMBER: _ClassVar[int]
    PAGE_FIELD_NUMBER: _ClassVar[int]
    include_revoked: bool
    include_expired: bool
    grantee_kind: str
    page: PageRequest
    def __init__(self, include_revoked: bool = ..., include_expired: bool = ..., grantee_kind: _Optional[str] = ..., page: _Optional[_Union[PageRequest, _Mapping]] = ...) -> None: ...

class ListGrantsResponse(_message.Message):
    __slots__ = ("grants", "page")
    GRANTS_FIELD_NUMBER: _ClassVar[int]
    PAGE_FIELD_NUMBER: _ClassVar[int]
    grants: _containers.RepeatedCompositeFieldContainer[Grant]
    page: PageResponse
    def __init__(self, grants: _Optional[_Iterable[_Union[Grant, _Mapping]]] = ..., page: _Optional[_Union[PageResponse, _Mapping]] = ...) -> None: ...

class UpdateGrantRequest(_message.Message):
    __slots__ = ("grant_ulid", "grantee_label", "expires_at_ms")
    GRANT_ULID_FIELD_NUMBER: _ClassVar[int]
    GRANTEE_LABEL_FIELD_NUMBER: _ClassVar[int]
    EXPIRES_AT_MS_FIELD_NUMBER: _ClassVar[int]
    grant_ulid: Ulid
    grantee_label: str
    expires_at_ms: int
    def __init__(self, grant_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., grantee_label: _Optional[str] = ..., expires_at_ms: _Optional[int] = ...) -> None: ...

class RevokeGrantRequest(_message.Message):
    __slots__ = ("grant_ulid", "reason")
    GRANT_ULID_FIELD_NUMBER: _ClassVar[int]
    REASON_FIELD_NUMBER: _ClassVar[int]
    grant_ulid: Ulid
    reason: str
    def __init__(self, grant_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., reason: _Optional[str] = ...) -> None: ...

class RevokeGrantResponse(_message.Message):
    __slots__ = ("revoked_at_ms",)
    REVOKED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    revoked_at_ms: int
    def __init__(self, revoked_at_ms: _Optional[int] = ...) -> None: ...

class Case(_message.Message):
    __slots__ = ("ulid", "case_type", "case_label", "started_at_ms", "ended_at_ms", "parent_case_ulid", "predecessor_case_ulid", "opening_authority_grant_ulid", "inactivity_close_after_h", "last_activity_at_ms")
    ULID_FIELD_NUMBER: _ClassVar[int]
    CASE_TYPE_FIELD_NUMBER: _ClassVar[int]
    CASE_LABEL_FIELD_NUMBER: _ClassVar[int]
    STARTED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    ENDED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    PARENT_CASE_ULID_FIELD_NUMBER: _ClassVar[int]
    PREDECESSOR_CASE_ULID_FIELD_NUMBER: _ClassVar[int]
    OPENING_AUTHORITY_GRANT_ULID_FIELD_NUMBER: _ClassVar[int]
    INACTIVITY_CLOSE_AFTER_H_FIELD_NUMBER: _ClassVar[int]
    LAST_ACTIVITY_AT_MS_FIELD_NUMBER: _ClassVar[int]
    ulid: Ulid
    case_type: str
    case_label: str
    started_at_ms: int
    ended_at_ms: int
    parent_case_ulid: Ulid
    predecessor_case_ulid: Ulid
    opening_authority_grant_ulid: Ulid
    inactivity_close_after_h: int
    last_activity_at_ms: int
    def __init__(self, ulid: _Optional[_Union[Ulid, _Mapping]] = ..., case_type: _Optional[str] = ..., case_label: _Optional[str] = ..., started_at_ms: _Optional[int] = ..., ended_at_ms: _Optional[int] = ..., parent_case_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., predecessor_case_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., opening_authority_grant_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., inactivity_close_after_h: _Optional[int] = ..., last_activity_at_ms: _Optional[int] = ...) -> None: ...

class CreateCaseRequest(_message.Message):
    __slots__ = ("case_type", "case_label", "parent_case_ulid", "predecessor_case_ulid", "inactivity_close_after_h", "initial_filters")
    CASE_TYPE_FIELD_NUMBER: _ClassVar[int]
    CASE_LABEL_FIELD_NUMBER: _ClassVar[int]
    PARENT_CASE_ULID_FIELD_NUMBER: _ClassVar[int]
    PREDECESSOR_CASE_ULID_FIELD_NUMBER: _ClassVar[int]
    INACTIVITY_CLOSE_AFTER_H_FIELD_NUMBER: _ClassVar[int]
    INITIAL_FILTERS_FIELD_NUMBER: _ClassVar[int]
    case_type: str
    case_label: str
    parent_case_ulid: Ulid
    predecessor_case_ulid: Ulid
    inactivity_close_after_h: int
    initial_filters: _containers.RepeatedCompositeFieldContainer[EventFilter]
    def __init__(self, case_type: _Optional[str] = ..., case_label: _Optional[str] = ..., parent_case_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., predecessor_case_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., inactivity_close_after_h: _Optional[int] = ..., initial_filters: _Optional[_Iterable[_Union[EventFilter, _Mapping]]] = ...) -> None: ...

class UpdateCaseRequest(_message.Message):
    __slots__ = ("case_ulid", "case_label", "parent_case_ulid", "predecessor_case_ulid", "inactivity_close_after_h")
    CASE_ULID_FIELD_NUMBER: _ClassVar[int]
    CASE_LABEL_FIELD_NUMBER: _ClassVar[int]
    PARENT_CASE_ULID_FIELD_NUMBER: _ClassVar[int]
    PREDECESSOR_CASE_ULID_FIELD_NUMBER: _ClassVar[int]
    INACTIVITY_CLOSE_AFTER_H_FIELD_NUMBER: _ClassVar[int]
    case_ulid: Ulid
    case_label: str
    parent_case_ulid: Ulid
    predecessor_case_ulid: Ulid
    inactivity_close_after_h: int
    def __init__(self, case_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., case_label: _Optional[str] = ..., parent_case_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., predecessor_case_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., inactivity_close_after_h: _Optional[int] = ...) -> None: ...

class CloseCaseRequest(_message.Message):
    __slots__ = ("case_ulid", "reason")
    CASE_ULID_FIELD_NUMBER: _ClassVar[int]
    REASON_FIELD_NUMBER: _ClassVar[int]
    case_ulid: Ulid
    reason: str
    def __init__(self, case_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., reason: _Optional[str] = ...) -> None: ...

class ReopenCaseRequest(_message.Message):
    __slots__ = ("case_reopen_token_ulid", "patient")
    CASE_REOPEN_TOKEN_ULID_FIELD_NUMBER: _ClassVar[int]
    PATIENT_FIELD_NUMBER: _ClassVar[int]
    case_reopen_token_ulid: Ulid
    patient: PatientReopen
    def __init__(self, case_reopen_token_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., patient: _Optional[_Union[PatientReopen, _Mapping]] = ...) -> None: ...

class PatientReopen(_message.Message):
    __slots__ = ("case_ulid",)
    CASE_ULID_FIELD_NUMBER: _ClassVar[int]
    case_ulid: Ulid
    def __init__(self, case_ulid: _Optional[_Union[Ulid, _Mapping]] = ...) -> None: ...

class ListCasesRequest(_message.Message):
    __slots__ = ("include_closed", "case_type", "page")
    INCLUDE_CLOSED_FIELD_NUMBER: _ClassVar[int]
    CASE_TYPE_FIELD_NUMBER: _ClassVar[int]
    PAGE_FIELD_NUMBER: _ClassVar[int]
    include_closed: bool
    case_type: str
    page: PageRequest
    def __init__(self, include_closed: bool = ..., case_type: _Optional[str] = ..., page: _Optional[_Union[PageRequest, _Mapping]] = ...) -> None: ...

class ListCasesResponse(_message.Message):
    __slots__ = ("cases", "page")
    CASES_FIELD_NUMBER: _ClassVar[int]
    PAGE_FIELD_NUMBER: _ClassVar[int]
    cases: _containers.RepeatedCompositeFieldContainer[Case]
    page: PageResponse
    def __init__(self, cases: _Optional[_Iterable[_Union[Case, _Mapping]]] = ..., page: _Optional[_Union[PageResponse, _Mapping]] = ...) -> None: ...

class GetCaseRequest(_message.Message):
    __slots__ = ("case_ulid",)
    CASE_ULID_FIELD_NUMBER: _ClassVar[int]
    case_ulid: Ulid
    def __init__(self, case_ulid: _Optional[_Union[Ulid, _Mapping]] = ...) -> None: ...

class CaseFilter(_message.Message):
    __slots__ = ("ulid", "case_ulid", "filter", "filter_label", "added_at_ms", "added_by_grant_ulid")
    ULID_FIELD_NUMBER: _ClassVar[int]
    CASE_ULID_FIELD_NUMBER: _ClassVar[int]
    FILTER_FIELD_NUMBER: _ClassVar[int]
    FILTER_LABEL_FIELD_NUMBER: _ClassVar[int]
    ADDED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    ADDED_BY_GRANT_ULID_FIELD_NUMBER: _ClassVar[int]
    ulid: Ulid
    case_ulid: Ulid
    filter: EventFilter
    filter_label: str
    added_at_ms: int
    added_by_grant_ulid: Ulid
    def __init__(self, ulid: _Optional[_Union[Ulid, _Mapping]] = ..., case_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., filter: _Optional[_Union[EventFilter, _Mapping]] = ..., filter_label: _Optional[str] = ..., added_at_ms: _Optional[int] = ..., added_by_grant_ulid: _Optional[_Union[Ulid, _Mapping]] = ...) -> None: ...

class AddCaseFilterRequest(_message.Message):
    __slots__ = ("case_ulid", "filter", "filter_label")
    CASE_ULID_FIELD_NUMBER: _ClassVar[int]
    FILTER_FIELD_NUMBER: _ClassVar[int]
    FILTER_LABEL_FIELD_NUMBER: _ClassVar[int]
    case_ulid: Ulid
    filter: EventFilter
    filter_label: str
    def __init__(self, case_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., filter: _Optional[_Union[EventFilter, _Mapping]] = ..., filter_label: _Optional[str] = ...) -> None: ...

class RemoveCaseFilterRequest(_message.Message):
    __slots__ = ("case_filter_ulid",)
    CASE_FILTER_ULID_FIELD_NUMBER: _ClassVar[int]
    case_filter_ulid: Ulid
    def __init__(self, case_filter_ulid: _Optional[_Union[Ulid, _Mapping]] = ...) -> None: ...

class RemoveCaseFilterResponse(_message.Message):
    __slots__ = ("removed_at_ms",)
    REMOVED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    removed_at_ms: int
    def __init__(self, removed_at_ms: _Optional[int] = ...) -> None: ...

class ListCaseFiltersRequest(_message.Message):
    __slots__ = ("case_ulid", "include_removed")
    CASE_ULID_FIELD_NUMBER: _ClassVar[int]
    INCLUDE_REMOVED_FIELD_NUMBER: _ClassVar[int]
    case_ulid: Ulid
    include_removed: bool
    def __init__(self, case_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., include_removed: bool = ...) -> None: ...

class ListCaseFiltersResponse(_message.Message):
    __slots__ = ("filters",)
    FILTERS_FIELD_NUMBER: _ClassVar[int]
    filters: _containers.RepeatedCompositeFieldContainer[CaseFilter]
    def __init__(self, filters: _Optional[_Iterable[_Union[CaseFilter, _Mapping]]] = ...) -> None: ...

class AuditQueryRequest(_message.Message):
    __slots__ = ("from_ms", "to_ms", "grant_ulid", "actor_type", "action", "result", "tail")
    FROM_MS_FIELD_NUMBER: _ClassVar[int]
    TO_MS_FIELD_NUMBER: _ClassVar[int]
    GRANT_ULID_FIELD_NUMBER: _ClassVar[int]
    ACTOR_TYPE_FIELD_NUMBER: _ClassVar[int]
    ACTION_FIELD_NUMBER: _ClassVar[int]
    RESULT_FIELD_NUMBER: _ClassVar[int]
    TAIL_FIELD_NUMBER: _ClassVar[int]
    from_ms: int
    to_ms: int
    grant_ulid: Ulid
    actor_type: str
    action: str
    result: str
    tail: bool
    def __init__(self, from_ms: _Optional[int] = ..., to_ms: _Optional[int] = ..., grant_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., actor_type: _Optional[str] = ..., action: _Optional[str] = ..., result: _Optional[str] = ..., tail: bool = ...) -> None: ...

class AuditEntry(_message.Message):
    __slots__ = ("ts_ms", "actor_type", "grant_ulid", "action", "query_kind", "query_params_json", "rows_returned", "rows_filtered", "result", "reason", "caller_ip", "caller_ua")
    TS_MS_FIELD_NUMBER: _ClassVar[int]
    ACTOR_TYPE_FIELD_NUMBER: _ClassVar[int]
    GRANT_ULID_FIELD_NUMBER: _ClassVar[int]
    ACTION_FIELD_NUMBER: _ClassVar[int]
    QUERY_KIND_FIELD_NUMBER: _ClassVar[int]
    QUERY_PARAMS_JSON_FIELD_NUMBER: _ClassVar[int]
    ROWS_RETURNED_FIELD_NUMBER: _ClassVar[int]
    ROWS_FILTERED_FIELD_NUMBER: _ClassVar[int]
    RESULT_FIELD_NUMBER: _ClassVar[int]
    REASON_FIELD_NUMBER: _ClassVar[int]
    CALLER_IP_FIELD_NUMBER: _ClassVar[int]
    CALLER_UA_FIELD_NUMBER: _ClassVar[int]
    ts_ms: int
    actor_type: str
    grant_ulid: Ulid
    action: str
    query_kind: str
    query_params_json: str
    rows_returned: int
    rows_filtered: int
    result: str
    reason: str
    caller_ip: str
    caller_ua: str
    def __init__(self, ts_ms: _Optional[int] = ..., actor_type: _Optional[str] = ..., grant_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., action: _Optional[str] = ..., query_kind: _Optional[str] = ..., query_params_json: _Optional[str] = ..., rows_returned: _Optional[int] = ..., rows_filtered: _Optional[int] = ..., result: _Optional[str] = ..., reason: _Optional[str] = ..., caller_ip: _Optional[str] = ..., caller_ua: _Optional[str] = ...) -> None: ...

class ListPendingRequest(_message.Message):
    __slots__ = ("submitting_grant_ulid", "status", "page")
    SUBMITTING_GRANT_ULID_FIELD_NUMBER: _ClassVar[int]
    STATUS_FIELD_NUMBER: _ClassVar[int]
    PAGE_FIELD_NUMBER: _ClassVar[int]
    submitting_grant_ulid: Ulid
    status: str
    page: PageRequest
    def __init__(self, submitting_grant_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., status: _Optional[str] = ..., page: _Optional[_Union[PageRequest, _Mapping]] = ...) -> None: ...

class ListPendingResponse(_message.Message):
    __slots__ = ("pending", "page")
    PENDING_FIELD_NUMBER: _ClassVar[int]
    PAGE_FIELD_NUMBER: _ClassVar[int]
    pending: _containers.RepeatedCompositeFieldContainer[PendingEvent]
    page: PageResponse
    def __init__(self, pending: _Optional[_Iterable[_Union[PendingEvent, _Mapping]]] = ..., page: _Optional[_Union[PageResponse, _Mapping]] = ...) -> None: ...

class PendingEvent(_message.Message):
    __slots__ = ("ulid", "submitted_at_ms", "submitting_grant_ulid", "event", "status", "reviewed_at_ms", "rejection_reason", "expires_at_ms", "approved_event_ulid")
    ULID_FIELD_NUMBER: _ClassVar[int]
    SUBMITTED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    SUBMITTING_GRANT_ULID_FIELD_NUMBER: _ClassVar[int]
    EVENT_FIELD_NUMBER: _ClassVar[int]
    STATUS_FIELD_NUMBER: _ClassVar[int]
    REVIEWED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    REJECTION_REASON_FIELD_NUMBER: _ClassVar[int]
    EXPIRES_AT_MS_FIELD_NUMBER: _ClassVar[int]
    APPROVED_EVENT_ULID_FIELD_NUMBER: _ClassVar[int]
    ulid: Ulid
    submitted_at_ms: int
    submitting_grant_ulid: Ulid
    event: Event
    status: str
    reviewed_at_ms: int
    rejection_reason: str
    expires_at_ms: int
    approved_event_ulid: Ulid
    def __init__(self, ulid: _Optional[_Union[Ulid, _Mapping]] = ..., submitted_at_ms: _Optional[int] = ..., submitting_grant_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., event: _Optional[_Union[Event, _Mapping]] = ..., status: _Optional[str] = ..., reviewed_at_ms: _Optional[int] = ..., rejection_reason: _Optional[str] = ..., expires_at_ms: _Optional[int] = ..., approved_event_ulid: _Optional[_Union[Ulid, _Mapping]] = ...) -> None: ...

class ApprovePendingRequest(_message.Message):
    __slots__ = ("pending_ulid", "also_auto_approve_this_type")
    PENDING_ULID_FIELD_NUMBER: _ClassVar[int]
    ALSO_AUTO_APPROVE_THIS_TYPE_FIELD_NUMBER: _ClassVar[int]
    pending_ulid: Ulid
    also_auto_approve_this_type: bool
    def __init__(self, pending_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., also_auto_approve_this_type: bool = ...) -> None: ...

class ApprovePendingResponse(_message.Message):
    __slots__ = ("event_ulid", "committed_at_ms")
    EVENT_ULID_FIELD_NUMBER: _ClassVar[int]
    COMMITTED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    event_ulid: Ulid
    committed_at_ms: int
    def __init__(self, event_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., committed_at_ms: _Optional[int] = ...) -> None: ...

class RejectPendingRequest(_message.Message):
    __slots__ = ("pending_ulid", "reason")
    PENDING_ULID_FIELD_NUMBER: _ClassVar[int]
    REASON_FIELD_NUMBER: _ClassVar[int]
    pending_ulid: Ulid
    reason: str
    def __init__(self, pending_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., reason: _Optional[str] = ...) -> None: ...

class RejectPendingResponse(_message.Message):
    __slots__ = ("rejected_at_ms",)
    REJECTED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    rejected_at_ms: int
    def __init__(self, rejected_at_ms: _Optional[int] = ...) -> None: ...

class PendingQuery(_message.Message):
    __slots__ = ("query_ulid", "grant_ulid", "query_kind", "query_payload", "requested_at_ms", "expires_at_ms", "decided_at_ms", "decision")
    QUERY_ULID_FIELD_NUMBER: _ClassVar[int]
    GRANT_ULID_FIELD_NUMBER: _ClassVar[int]
    QUERY_KIND_FIELD_NUMBER: _ClassVar[int]
    QUERY_PAYLOAD_FIELD_NUMBER: _ClassVar[int]
    REQUESTED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    EXPIRES_AT_MS_FIELD_NUMBER: _ClassVar[int]
    DECIDED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    DECISION_FIELD_NUMBER: _ClassVar[int]
    query_ulid: Ulid
    grant_ulid: Ulid
    query_kind: PendingQueryKind
    query_payload: bytes
    requested_at_ms: int
    expires_at_ms: int
    decided_at_ms: int
    decision: PendingQueryDecision
    def __init__(self, query_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., grant_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., query_kind: _Optional[_Union[PendingQueryKind, str]] = ..., query_payload: _Optional[bytes] = ..., requested_at_ms: _Optional[int] = ..., expires_at_ms: _Optional[int] = ..., decided_at_ms: _Optional[int] = ..., decision: _Optional[_Union[PendingQueryDecision, str]] = ...) -> None: ...

class ListPendingQueriesRequest(_message.Message):
    __slots__ = ("include_decided", "since_ms")
    INCLUDE_DECIDED_FIELD_NUMBER: _ClassVar[int]
    SINCE_MS_FIELD_NUMBER: _ClassVar[int]
    include_decided: bool
    since_ms: int
    def __init__(self, include_decided: bool = ..., since_ms: _Optional[int] = ...) -> None: ...

class ApprovePendingQueryRequest(_message.Message):
    __slots__ = ("query_ulid",)
    QUERY_ULID_FIELD_NUMBER: _ClassVar[int]
    query_ulid: Ulid
    def __init__(self, query_ulid: _Optional[_Union[Ulid, _Mapping]] = ...) -> None: ...

class ApprovePendingQueryResponse(_message.Message):
    __slots__ = ("ok",)
    OK_FIELD_NUMBER: _ClassVar[int]
    ok: bool
    def __init__(self, ok: bool = ...) -> None: ...

class RejectPendingQueryRequest(_message.Message):
    __slots__ = ("query_ulid", "reason")
    QUERY_ULID_FIELD_NUMBER: _ClassVar[int]
    REASON_FIELD_NUMBER: _ClassVar[int]
    query_ulid: Ulid
    reason: str
    def __init__(self, query_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., reason: _Optional[str] = ...) -> None: ...

class RejectPendingQueryResponse(_message.Message):
    __slots__ = ("ok",)
    OK_FIELD_NUMBER: _ClassVar[int]
    ok: bool
    def __init__(self, ok: bool = ...) -> None: ...

class ExportRequest(_message.Message):
    __slots__ = ("from_ms", "to_ms", "include_event_types", "encrypt_to_passphrase", "resume_token")
    FROM_MS_FIELD_NUMBER: _ClassVar[int]
    TO_MS_FIELD_NUMBER: _ClassVar[int]
    INCLUDE_EVENT_TYPES_FIELD_NUMBER: _ClassVar[int]
    ENCRYPT_TO_PASSPHRASE_FIELD_NUMBER: _ClassVar[int]
    RESUME_TOKEN_FIELD_NUMBER: _ClassVar[int]
    from_ms: int
    to_ms: int
    include_event_types: _containers.RepeatedScalarFieldContainer[str]
    encrypt_to_passphrase: str
    resume_token: str
    def __init__(self, from_ms: _Optional[int] = ..., to_ms: _Optional[int] = ..., include_event_types: _Optional[_Iterable[str]] = ..., encrypt_to_passphrase: _Optional[str] = ..., resume_token: _Optional[str] = ...) -> None: ...

class ExportChunk(_message.Message):
    __slots__ = ("init", "frame", "finish")
    INIT_FIELD_NUMBER: _ClassVar[int]
    FRAME_FIELD_NUMBER: _ClassVar[int]
    FINISH_FIELD_NUMBER: _ClassVar[int]
    init: ExportInit
    frame: ExportFrame
    finish: ExportFinish
    def __init__(self, init: _Optional[_Union[ExportInit, _Mapping]] = ..., frame: _Optional[_Union[ExportFrame, _Mapping]] = ..., finish: _Optional[_Union[ExportFinish, _Mapping]] = ...) -> None: ...

class ExportInit(_message.Message):
    __slots__ = ("format_version", "source_instance_pubkey_hex", "encryption")
    FORMAT_VERSION_FIELD_NUMBER: _ClassVar[int]
    SOURCE_INSTANCE_PUBKEY_HEX_FIELD_NUMBER: _ClassVar[int]
    ENCRYPTION_FIELD_NUMBER: _ClassVar[int]
    format_version: str
    source_instance_pubkey_hex: str
    encryption: _struct_pb2.Struct
    def __init__(self, format_version: _Optional[str] = ..., source_instance_pubkey_hex: _Optional[str] = ..., encryption: _Optional[_Union[_struct_pb2.Struct, _Mapping]] = ...) -> None: ...

class ExportFrame(_message.Message):
    __slots__ = ("event", "grant", "audit_entry", "attachment", "pending", "device", "app_version", "registry_entry", "peer_sync")
    EVENT_FIELD_NUMBER: _ClassVar[int]
    GRANT_FIELD_NUMBER: _ClassVar[int]
    AUDIT_ENTRY_FIELD_NUMBER: _ClassVar[int]
    ATTACHMENT_FIELD_NUMBER: _ClassVar[int]
    PENDING_FIELD_NUMBER: _ClassVar[int]
    DEVICE_FIELD_NUMBER: _ClassVar[int]
    APP_VERSION_FIELD_NUMBER: _ClassVar[int]
    REGISTRY_ENTRY_FIELD_NUMBER: _ClassVar[int]
    PEER_SYNC_FIELD_NUMBER: _ClassVar[int]
    event: Event
    grant: Grant
    audit_entry: AuditEntry
    attachment: AttachmentBlob
    pending: PendingEvent
    device: DeviceRow
    app_version: AppVersionRow
    registry_entry: RegistryEntry
    peer_sync: PeerSyncRow
    def __init__(self, event: _Optional[_Union[Event, _Mapping]] = ..., grant: _Optional[_Union[Grant, _Mapping]] = ..., audit_entry: _Optional[_Union[AuditEntry, _Mapping]] = ..., attachment: _Optional[_Union[AttachmentBlob, _Mapping]] = ..., pending: _Optional[_Union[PendingEvent, _Mapping]] = ..., device: _Optional[_Union[DeviceRow, _Mapping]] = ..., app_version: _Optional[_Union[AppVersionRow, _Mapping]] = ..., registry_entry: _Optional[_Union[RegistryEntry, _Mapping]] = ..., peer_sync: _Optional[_Union[PeerSyncRow, _Mapping]] = ...) -> None: ...

class ExportFinish(_message.Message):
    __slots__ = ("resume_token", "signature", "source_instance_pubkey_hex")
    RESUME_TOKEN_FIELD_NUMBER: _ClassVar[int]
    SIGNATURE_FIELD_NUMBER: _ClassVar[int]
    SOURCE_INSTANCE_PUBKEY_HEX_FIELD_NUMBER: _ClassVar[int]
    resume_token: str
    signature: bytes
    source_instance_pubkey_hex: str
    def __init__(self, resume_token: _Optional[str] = ..., signature: _Optional[bytes] = ..., source_instance_pubkey_hex: _Optional[str] = ...) -> None: ...

class ImportChunk(_message.Message):
    __slots__ = ("init", "frame", "finish")
    INIT_FIELD_NUMBER: _ClassVar[int]
    FRAME_FIELD_NUMBER: _ClassVar[int]
    FINISH_FIELD_NUMBER: _ClassVar[int]
    init: ImportInit
    frame: ExportFrame
    finish: ImportFinish
    def __init__(self, init: _Optional[_Union[ImportInit, _Mapping]] = ..., frame: _Optional[_Union[ExportFrame, _Mapping]] = ..., finish: _Optional[_Union[ImportFinish, _Mapping]] = ...) -> None: ...

class ImportInit(_message.Message):
    __slots__ = ("source_instance_pubkey_hex",)
    SOURCE_INSTANCE_PUBKEY_HEX_FIELD_NUMBER: _ClassVar[int]
    source_instance_pubkey_hex: str
    def __init__(self, source_instance_pubkey_hex: _Optional[str] = ...) -> None: ...

class ImportFinish(_message.Message):
    __slots__ = ("signature",)
    SIGNATURE_FIELD_NUMBER: _ClassVar[int]
    signature: bytes
    def __init__(self, signature: _Optional[bytes] = ...) -> None: ...

class ImportResponse(_message.Message):
    __slots__ = ("events_imported", "grants_imported", "audit_entries_imported", "warnings", "unknown_extensions")
    EVENTS_IMPORTED_FIELD_NUMBER: _ClassVar[int]
    GRANTS_IMPORTED_FIELD_NUMBER: _ClassVar[int]
    AUDIT_ENTRIES_IMPORTED_FIELD_NUMBER: _ClassVar[int]
    WARNINGS_FIELD_NUMBER: _ClassVar[int]
    UNKNOWN_EXTENSIONS_FIELD_NUMBER: _ClassVar[int]
    events_imported: int
    grants_imported: int
    audit_entries_imported: int
    warnings: _containers.RepeatedScalarFieldContainer[str]
    unknown_extensions: _containers.RepeatedCompositeFieldContainer[UnknownExtension]
    def __init__(self, events_imported: _Optional[int] = ..., grants_imported: _Optional[int] = ..., audit_entries_imported: _Optional[int] = ..., warnings: _Optional[_Iterable[str]] = ..., unknown_extensions: _Optional[_Iterable[_Union[UnknownExtension, _Mapping]]] = ...) -> None: ...

class UnknownExtension(_message.Message):
    __slots__ = ("namespace", "preserved_as", "entries")
    NAMESPACE_FIELD_NUMBER: _ClassVar[int]
    PRESERVED_AS_FIELD_NUMBER: _ClassVar[int]
    ENTRIES_FIELD_NUMBER: _ClassVar[int]
    namespace: str
    preserved_as: str
    entries: int
    def __init__(self, namespace: _Optional[str] = ..., preserved_as: _Optional[str] = ..., entries: _Optional[int] = ...) -> None: ...

class AttachmentBlob(_message.Message):
    __slots__ = ("ref", "data")
    REF_FIELD_NUMBER: _ClassVar[int]
    DATA_FIELD_NUMBER: _ClassVar[int]
    ref: AttachmentRef
    data: bytes
    def __init__(self, ref: _Optional[_Union[AttachmentRef, _Mapping]] = ..., data: _Optional[bytes] = ...) -> None: ...

class DeviceRow(_message.Message):
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

class AppVersionRow(_message.Message):
    __slots__ = ("app_name", "version", "platform")
    APP_NAME_FIELD_NUMBER: _ClassVar[int]
    VERSION_FIELD_NUMBER: _ClassVar[int]
    PLATFORM_FIELD_NUMBER: _ClassVar[int]
    app_name: str
    version: str
    platform: str
    def __init__(self, app_name: _Optional[str] = ..., version: _Optional[str] = ..., platform: _Optional[str] = ...) -> None: ...

class RegistryEntry(_message.Message):
    __slots__ = ("kind", "payload")
    KIND_FIELD_NUMBER: _ClassVar[int]
    PAYLOAD_FIELD_NUMBER: _ClassVar[int]
    kind: str
    payload: _struct_pb2.Struct
    def __init__(self, kind: _Optional[str] = ..., payload: _Optional[_Union[_struct_pb2.Struct, _Mapping]] = ...) -> None: ...

class PeerSyncRow(_message.Message):
    __slots__ = ("peer_label", "peer_kind", "peer_ulid", "last_outbound_rowid", "last_inbound_peer_rowid")
    PEER_LABEL_FIELD_NUMBER: _ClassVar[int]
    PEER_KIND_FIELD_NUMBER: _ClassVar[int]
    PEER_ULID_FIELD_NUMBER: _ClassVar[int]
    LAST_OUTBOUND_ROWID_FIELD_NUMBER: _ClassVar[int]
    LAST_INBOUND_PEER_ROWID_FIELD_NUMBER: _ClassVar[int]
    peer_label: str
    peer_kind: str
    peer_ulid: Ulid
    last_outbound_rowid: int
    last_inbound_peer_rowid: int
    def __init__(self, peer_label: _Optional[str] = ..., peer_kind: _Optional[str] = ..., peer_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., last_outbound_rowid: _Optional[int] = ..., last_inbound_peer_rowid: _Optional[int] = ...) -> None: ...

class RegisterSignerRequest(_message.Message):
    __slots__ = ("signer_kid", "signer_label", "sig_alg", "public_key_pem")
    SIGNER_KID_FIELD_NUMBER: _ClassVar[int]
    SIGNER_LABEL_FIELD_NUMBER: _ClassVar[int]
    SIG_ALG_FIELD_NUMBER: _ClassVar[int]
    PUBLIC_KEY_PEM_FIELD_NUMBER: _ClassVar[int]
    signer_kid: str
    signer_label: str
    sig_alg: str
    public_key_pem: str
    def __init__(self, signer_kid: _Optional[str] = ..., signer_label: _Optional[str] = ..., sig_alg: _Optional[str] = ..., public_key_pem: _Optional[str] = ...) -> None: ...

class RegisterSignerResponse(_message.Message):
    __slots__ = ("signer", "registered_at_ms")
    SIGNER_FIELD_NUMBER: _ClassVar[int]
    REGISTERED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    signer: SignerInfo
    registered_at_ms: int
    def __init__(self, signer: _Optional[_Union[SignerInfo, _Mapping]] = ..., registered_at_ms: _Optional[int] = ...) -> None: ...

class ListSignersRequest(_message.Message):
    __slots__ = ("include_revoked",)
    INCLUDE_REVOKED_FIELD_NUMBER: _ClassVar[int]
    include_revoked: bool
    def __init__(self, include_revoked: bool = ...) -> None: ...

class ListSignersResponse(_message.Message):
    __slots__ = ("signers",)
    SIGNERS_FIELD_NUMBER: _ClassVar[int]
    signers: _containers.RepeatedCompositeFieldContainer[SignerInfo]
    def __init__(self, signers: _Optional[_Iterable[_Union[SignerInfo, _Mapping]]] = ...) -> None: ...

class RevokeSignerRequest(_message.Message):
    __slots__ = ("signer_kid",)
    SIGNER_KID_FIELD_NUMBER: _ClassVar[int]
    signer_kid: str
    def __init__(self, signer_kid: _Optional[str] = ...) -> None: ...

class RevokeSignerResponse(_message.Message):
    __slots__ = ("revoked_at_ms",)
    REVOKED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    revoked_at_ms: int
    def __init__(self, revoked_at_ms: _Optional[int] = ...) -> None: ...

class WhoAmIRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class WhoAmIResponse(_message.Message):
    __slots__ = ("user_ulid", "token_kind", "grant_ulid", "grantee_label", "effective_grant", "caller_ip", "device_label", "linked_identities")
    USER_ULID_FIELD_NUMBER: _ClassVar[int]
    TOKEN_KIND_FIELD_NUMBER: _ClassVar[int]
    GRANT_ULID_FIELD_NUMBER: _ClassVar[int]
    GRANTEE_LABEL_FIELD_NUMBER: _ClassVar[int]
    EFFECTIVE_GRANT_FIELD_NUMBER: _ClassVar[int]
    CALLER_IP_FIELD_NUMBER: _ClassVar[int]
    DEVICE_LABEL_FIELD_NUMBER: _ClassVar[int]
    LINKED_IDENTITIES_FIELD_NUMBER: _ClassVar[int]
    user_ulid: Ulid
    token_kind: str
    grant_ulid: Ulid
    grantee_label: str
    effective_grant: Grant
    caller_ip: str
    device_label: str
    linked_identities: _containers.RepeatedCompositeFieldContainer[LinkedIdentitySummary]
    def __init__(self, user_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., token_kind: _Optional[str] = ..., grant_ulid: _Optional[_Union[Ulid, _Mapping]] = ..., grantee_label: _Optional[str] = ..., effective_grant: _Optional[_Union[Grant, _Mapping]] = ..., caller_ip: _Optional[str] = ..., device_label: _Optional[str] = ..., linked_identities: _Optional[_Iterable[_Union[LinkedIdentitySummary, _Mapping]]] = ...) -> None: ...

class LinkedIdentitySummary(_message.Message):
    __slots__ = ("provider", "display_label", "is_primary", "linked_at_ms")
    PROVIDER_FIELD_NUMBER: _ClassVar[int]
    DISPLAY_LABEL_FIELD_NUMBER: _ClassVar[int]
    IS_PRIMARY_FIELD_NUMBER: _ClassVar[int]
    LINKED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    provider: str
    display_label: str
    is_primary: bool
    linked_at_ms: int
    def __init__(self, provider: _Optional[str] = ..., display_label: _Optional[str] = ..., is_primary: bool = ..., linked_at_ms: _Optional[int] = ...) -> None: ...

class HealthRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class HealthResponse(_message.Message):
    __slots__ = ("status", "server_time_ms", "server_version", "protocol_version", "registry_version", "subsystems")
    class SubsystemsEntry(_message.Message):
        __slots__ = ("key", "value")
        KEY_FIELD_NUMBER: _ClassVar[int]
        VALUE_FIELD_NUMBER: _ClassVar[int]
        key: str
        value: str
        def __init__(self, key: _Optional[str] = ..., value: _Optional[str] = ...) -> None: ...
    STATUS_FIELD_NUMBER: _ClassVar[int]
    SERVER_TIME_MS_FIELD_NUMBER: _ClassVar[int]
    SERVER_VERSION_FIELD_NUMBER: _ClassVar[int]
    PROTOCOL_VERSION_FIELD_NUMBER: _ClassVar[int]
    REGISTRY_VERSION_FIELD_NUMBER: _ClassVar[int]
    SUBSYSTEMS_FIELD_NUMBER: _ClassVar[int]
    status: str
    server_time_ms: int
    server_version: str
    protocol_version: str
    registry_version: int
    subsystems: _containers.ScalarMap[str, str]
    def __init__(self, status: _Optional[str] = ..., server_time_ms: _Optional[int] = ..., server_version: _Optional[str] = ..., protocol_version: _Optional[str] = ..., registry_version: _Optional[int] = ..., subsystems: _Optional[_Mapping[str, str]] = ...) -> None: ...
