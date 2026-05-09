# 001_event_type_deny

Asserts that a grant with `default_action=allow` and an `event_type` deny
rule (against `std.heart_rate_resting`) silently drops the denied type from
the result set: 3 events written, 2 returned, 1 in `rows_filtered`. Per
spec/storage-format.md "Combination precedence", deny wins on conflict.
