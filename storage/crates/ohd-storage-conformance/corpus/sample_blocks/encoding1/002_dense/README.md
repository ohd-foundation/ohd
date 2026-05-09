# 002_dense

Encoding 1 byte-determinism on a 60-sample heart-rate-style stream
(1Hz cadence, smooth values). Validates that the codec handles a
typical OHD sample block (~15 min @ 4 samples/min for HR_resting,
or 1Hz for HR_series) deterministically.
