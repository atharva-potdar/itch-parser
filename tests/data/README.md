# Real capture fixtures

Drop a Nasdaq historical TotalView-ITCH file here to enable the
`real_capture_parses_without_errors` test in `tests/spec_conformance.rs`.

Expected format: the raw, **gunzipped** Nasdaq capture — a stream of 2-byte
big-endian length prefixes, each followed by exactly one ITCH 5.0 message, with
no MoldUDP64 header. That is what Nasdaq publishes, e.g.:

```
01302019.NASDAQ_ITCH50
```

Do not commit captures: they are large and licensed. The test skips itself
(printing a notice) when no matching file is present, so the suite stays
hermetic and CI-safe.

A filename containing `ITCH50`, or with the `.itch` extension, is picked up.
