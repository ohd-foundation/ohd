from ohdc.v0 import ohdc_pb2 as _ohdc_pb2
from google.protobuf.internal import containers as _containers
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class ListIdentitiesRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class ListIdentitiesResponse(_message.Message):
    __slots__ = ("identities",)
    IDENTITIES_FIELD_NUMBER: _ClassVar[int]
    identities: _containers.RepeatedCompositeFieldContainer[Identity]
    def __init__(self, identities: _Optional[_Iterable[_Union[Identity, _Mapping]]] = ...) -> None: ...

class Identity(_message.Message):
    __slots__ = ("provider", "subject", "email", "linked_at_ms", "display_label", "is_primary", "last_login_ms")
    PROVIDER_FIELD_NUMBER: _ClassVar[int]
    SUBJECT_FIELD_NUMBER: _ClassVar[int]
    EMAIL_FIELD_NUMBER: _ClassVar[int]
    LINKED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    DISPLAY_LABEL_FIELD_NUMBER: _ClassVar[int]
    IS_PRIMARY_FIELD_NUMBER: _ClassVar[int]
    LAST_LOGIN_MS_FIELD_NUMBER: _ClassVar[int]
    provider: str
    subject: str
    email: str
    linked_at_ms: int
    display_label: str
    is_primary: bool
    last_login_ms: int
    def __init__(self, provider: _Optional[str] = ..., subject: _Optional[str] = ..., email: _Optional[str] = ..., linked_at_ms: _Optional[int] = ..., display_label: _Optional[str] = ..., is_primary: bool = ..., last_login_ms: _Optional[int] = ...) -> None: ...

class LinkIdentityStartRequest(_message.Message):
    __slots__ = ("provider_hint",)
    PROVIDER_HINT_FIELD_NUMBER: _ClassVar[int]
    provider_hint: str
    def __init__(self, provider_hint: _Optional[str] = ...) -> None: ...

class LinkIdentityStartResponse(_message.Message):
    __slots__ = ("link_token", "oauth_url", "expires_at_ms")
    LINK_TOKEN_FIELD_NUMBER: _ClassVar[int]
    OAUTH_URL_FIELD_NUMBER: _ClassVar[int]
    EXPIRES_AT_MS_FIELD_NUMBER: _ClassVar[int]
    link_token: str
    oauth_url: str
    expires_at_ms: int
    def __init__(self, link_token: _Optional[str] = ..., oauth_url: _Optional[str] = ..., expires_at_ms: _Optional[int] = ...) -> None: ...

class CompleteIdentityLinkRequest(_message.Message):
    __slots__ = ("link_token", "id_token", "issuer", "audiences", "display_label")
    LINK_TOKEN_FIELD_NUMBER: _ClassVar[int]
    ID_TOKEN_FIELD_NUMBER: _ClassVar[int]
    ISSUER_FIELD_NUMBER: _ClassVar[int]
    AUDIENCES_FIELD_NUMBER: _ClassVar[int]
    DISPLAY_LABEL_FIELD_NUMBER: _ClassVar[int]
    link_token: str
    id_token: str
    issuer: str
    audiences: _containers.RepeatedScalarFieldContainer[str]
    display_label: str
    def __init__(self, link_token: _Optional[str] = ..., id_token: _Optional[str] = ..., issuer: _Optional[str] = ..., audiences: _Optional[_Iterable[str]] = ..., display_label: _Optional[str] = ...) -> None: ...

class CompleteIdentityLinkResponse(_message.Message):
    __slots__ = ("identity",)
    IDENTITY_FIELD_NUMBER: _ClassVar[int]
    identity: Identity
    def __init__(self, identity: _Optional[_Union[Identity, _Mapping]] = ...) -> None: ...

class UnlinkIdentityRequest(_message.Message):
    __slots__ = ("provider", "subject")
    PROVIDER_FIELD_NUMBER: _ClassVar[int]
    SUBJECT_FIELD_NUMBER: _ClassVar[int]
    provider: str
    subject: str
    def __init__(self, provider: _Optional[str] = ..., subject: _Optional[str] = ...) -> None: ...

class UnlinkIdentityResponse(_message.Message):
    __slots__ = ("unlinked_at_ms",)
    UNLINKED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    unlinked_at_ms: int
    def __init__(self, unlinked_at_ms: _Optional[int] = ...) -> None: ...

class SetPrimaryIdentityRequest(_message.Message):
    __slots__ = ("provider", "subject")
    PROVIDER_FIELD_NUMBER: _ClassVar[int]
    SUBJECT_FIELD_NUMBER: _ClassVar[int]
    provider: str
    subject: str
    def __init__(self, provider: _Optional[str] = ..., subject: _Optional[str] = ...) -> None: ...

class SetPrimaryIdentityResponse(_message.Message):
    __slots__ = ("updated_at_ms",)
    UPDATED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    updated_at_ms: int
    def __init__(self, updated_at_ms: _Optional[int] = ...) -> None: ...

class ListSessionsRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class ListSessionsResponse(_message.Message):
    __slots__ = ("sessions",)
    SESSIONS_FIELD_NUMBER: _ClassVar[int]
    sessions: _containers.RepeatedCompositeFieldContainer[SessionInfo]
    def __init__(self, sessions: _Optional[_Iterable[_Union[SessionInfo, _Mapping]]] = ...) -> None: ...

class SessionInfo(_message.Message):
    __slots__ = ("session_ulid", "created_at_ms", "last_seen_ms", "user_agent", "ip_origin")
    SESSION_ULID_FIELD_NUMBER: _ClassVar[int]
    CREATED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    LAST_SEEN_MS_FIELD_NUMBER: _ClassVar[int]
    USER_AGENT_FIELD_NUMBER: _ClassVar[int]
    IP_ORIGIN_FIELD_NUMBER: _ClassVar[int]
    session_ulid: _ohdc_pb2.Ulid
    created_at_ms: int
    last_seen_ms: int
    user_agent: str
    ip_origin: str
    def __init__(self, session_ulid: _Optional[_Union[_ohdc_pb2.Ulid, _Mapping]] = ..., created_at_ms: _Optional[int] = ..., last_seen_ms: _Optional[int] = ..., user_agent: _Optional[str] = ..., ip_origin: _Optional[str] = ...) -> None: ...

class RevokeSessionRequest(_message.Message):
    __slots__ = ("session_ulid",)
    SESSION_ULID_FIELD_NUMBER: _ClassVar[int]
    session_ulid: _ohdc_pb2.Ulid
    def __init__(self, session_ulid: _Optional[_Union[_ohdc_pb2.Ulid, _Mapping]] = ...) -> None: ...

class RevokeSessionResponse(_message.Message):
    __slots__ = ("revoked_at_ms",)
    REVOKED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    revoked_at_ms: int
    def __init__(self, revoked_at_ms: _Optional[int] = ...) -> None: ...

class LogoutRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class LogoutResponse(_message.Message):
    __slots__ = ("logged_out_at_ms",)
    LOGGED_OUT_AT_MS_FIELD_NUMBER: _ClassVar[int]
    logged_out_at_ms: int
    def __init__(self, logged_out_at_ms: _Optional[int] = ...) -> None: ...

class LogoutEverywhereRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class LogoutEverywhereResponse(_message.Message):
    __slots__ = ("sessions_revoked", "logged_out_at_ms")
    SESSIONS_REVOKED_FIELD_NUMBER: _ClassVar[int]
    LOGGED_OUT_AT_MS_FIELD_NUMBER: _ClassVar[int]
    sessions_revoked: int
    logged_out_at_ms: int
    def __init__(self, sessions_revoked: _Optional[int] = ..., logged_out_at_ms: _Optional[int] = ...) -> None: ...

class IssueInviteRequest(_message.Message):
    __slots__ = ("email_bound", "expires_at_ms", "note")
    EMAIL_BOUND_FIELD_NUMBER: _ClassVar[int]
    EXPIRES_AT_MS_FIELD_NUMBER: _ClassVar[int]
    NOTE_FIELD_NUMBER: _ClassVar[int]
    email_bound: str
    expires_at_ms: int
    note: str
    def __init__(self, email_bound: _Optional[str] = ..., expires_at_ms: _Optional[int] = ..., note: _Optional[str] = ...) -> None: ...

class IssueInviteResponse(_message.Message):
    __slots__ = ("invite_token", "redeem_url")
    INVITE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    REDEEM_URL_FIELD_NUMBER: _ClassVar[int]
    invite_token: str
    redeem_url: str
    def __init__(self, invite_token: _Optional[str] = ..., redeem_url: _Optional[str] = ...) -> None: ...

class ListInvitesRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class ListInvitesResponse(_message.Message):
    __slots__ = ("invites",)
    INVITES_FIELD_NUMBER: _ClassVar[int]
    invites: _containers.RepeatedCompositeFieldContainer[InviteInfo]
    def __init__(self, invites: _Optional[_Iterable[_Union[InviteInfo, _Mapping]]] = ...) -> None: ...

class InviteInfo(_message.Message):
    __slots__ = ("ulid", "email_bound", "issued_at_ms", "redeemed_at_ms", "expires_at_ms", "revoked_at_ms")
    ULID_FIELD_NUMBER: _ClassVar[int]
    EMAIL_BOUND_FIELD_NUMBER: _ClassVar[int]
    ISSUED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    REDEEMED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    EXPIRES_AT_MS_FIELD_NUMBER: _ClassVar[int]
    REVOKED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    ulid: _ohdc_pb2.Ulid
    email_bound: str
    issued_at_ms: int
    redeemed_at_ms: int
    expires_at_ms: int
    revoked_at_ms: int
    def __init__(self, ulid: _Optional[_Union[_ohdc_pb2.Ulid, _Mapping]] = ..., email_bound: _Optional[str] = ..., issued_at_ms: _Optional[int] = ..., redeemed_at_ms: _Optional[int] = ..., expires_at_ms: _Optional[int] = ..., revoked_at_ms: _Optional[int] = ...) -> None: ...

class RevokeInviteRequest(_message.Message):
    __slots__ = ("ulid",)
    ULID_FIELD_NUMBER: _ClassVar[int]
    ulid: _ohdc_pb2.Ulid
    def __init__(self, ulid: _Optional[_Union[_ohdc_pb2.Ulid, _Mapping]] = ...) -> None: ...

class RevokeInviteResponse(_message.Message):
    __slots__ = ("revoked_at_ms",)
    REVOKED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    revoked_at_ms: int
    def __init__(self, revoked_at_ms: _Optional[int] = ...) -> None: ...

class IssueDeviceTokenRequest(_message.Message):
    __slots__ = ("device_label", "device_kind", "event_types")
    DEVICE_LABEL_FIELD_NUMBER: _ClassVar[int]
    DEVICE_KIND_FIELD_NUMBER: _ClassVar[int]
    EVENT_TYPES_FIELD_NUMBER: _ClassVar[int]
    device_label: str
    device_kind: str
    event_types: _containers.RepeatedScalarFieldContainer[str]
    def __init__(self, device_label: _Optional[str] = ..., device_kind: _Optional[str] = ..., event_types: _Optional[_Iterable[str]] = ...) -> None: ...

class IssueDeviceTokenResponse(_message.Message):
    __slots__ = ("token", "grant_ulid")
    TOKEN_FIELD_NUMBER: _ClassVar[int]
    GRANT_ULID_FIELD_NUMBER: _ClassVar[int]
    token: str
    grant_ulid: _ohdc_pb2.Ulid
    def __init__(self, token: _Optional[str] = ..., grant_ulid: _Optional[_Union[_ohdc_pb2.Ulid, _Mapping]] = ...) -> None: ...

class RegisterPushTokenRequest(_message.Message):
    __slots__ = ("platform", "token")
    PLATFORM_FIELD_NUMBER: _ClassVar[int]
    TOKEN_FIELD_NUMBER: _ClassVar[int]
    platform: str
    token: str
    def __init__(self, platform: _Optional[str] = ..., token: _Optional[str] = ...) -> None: ...

class RegisterPushTokenResponse(_message.Message):
    __slots__ = ("registered_at_ms",)
    REGISTERED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    registered_at_ms: int
    def __init__(self, registered_at_ms: _Optional[int] = ...) -> None: ...

class UpdateNotificationPreferencesRequest(_message.Message):
    __slots__ = ("quiet_hours_enabled", "quiet_hours_start", "quiet_hours_end", "quiet_hours_tz")
    QUIET_HOURS_ENABLED_FIELD_NUMBER: _ClassVar[int]
    QUIET_HOURS_START_FIELD_NUMBER: _ClassVar[int]
    QUIET_HOURS_END_FIELD_NUMBER: _ClassVar[int]
    QUIET_HOURS_TZ_FIELD_NUMBER: _ClassVar[int]
    quiet_hours_enabled: bool
    quiet_hours_start: int
    quiet_hours_end: int
    quiet_hours_tz: str
    def __init__(self, quiet_hours_enabled: bool = ..., quiet_hours_start: _Optional[int] = ..., quiet_hours_end: _Optional[int] = ..., quiet_hours_tz: _Optional[str] = ...) -> None: ...

class UpdateNotificationPreferencesResponse(_message.Message):
    __slots__ = ("updated_at_ms",)
    UPDATED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    updated_at_ms: int
    def __init__(self, updated_at_ms: _Optional[int] = ...) -> None: ...
