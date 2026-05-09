# 001_self_session_round_trip

Self-session writes two events of different types and reads them back via
QueryEvents with a tight time-window filter. Asserts that:

- Both events round-trip,
- They appear in `TIME_DESC` order (newer event first),
- Self-session sees both regardless of type (no grant scope intersection).
