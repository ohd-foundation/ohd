# 001_simple

Asserts encoding 2 (delta-zigzag-varint timestamps + int16 quantized values
+ scale + zstd) is byte-deterministic on a small 4-sample block at scale
1.0 (no quantization loss for integer-valued HR samples). Encoding 2 is
optional for v0 conformance but recommended; spec/storage-format.md says
"Implementations MUST support encoding 1 for read. Encoding 2 is strongly
recommended."
