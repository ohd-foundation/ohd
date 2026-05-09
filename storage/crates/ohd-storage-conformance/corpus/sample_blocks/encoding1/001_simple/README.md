# 001_simple

Asserts that encoding 1 (delta-zigzag-varint timestamps + float32 values,
zstd-compressed) is byte-deterministic for a small 4-sample block. The fixture
proves the encoder produces identical output on repeated runs and across
implementations targeting OHDC v0.
