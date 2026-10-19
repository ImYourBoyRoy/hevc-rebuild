# Measured performance — 2026-10-12

Evidence is synthetic and private under `/tmp/hevc-tasklist-20261011-00zltisa`; it is not packaged or real-library proof.

| Measurement | Result | Scope |
|---|---|---|
| Native scan 1 / 20 / 100 files | 0.775 / 4.296 / 19.745 s | Generated 320×180 SDR; serial |
| 100-file forecasts | 48 ms | Rebuild and eager YouTube forecasts |
| Native scan cancellation | 3.3 ms to exit | Owned CLI process session; no cooperative GUI cancel claim |
| Repeated 20-file scan | 4.335 s cold / 0.560 s warm | Same native process, 20 successful-profile cache hits |
| Changed source | 19 hits; changed file Analysis failed | No stale profile reuse |
| Prototype profile pool 1 / 2 / 4 workers | 3.648 / 2.016 / 1.153 s | Twenty generated clips; native helpers |
| Pool cancellation | 3.0 / 2.6 / 5.8 ms to join | Cancellation during probing |
| Injected pressure | 3 / 4 / 6 completed jobs | Stops scheduling after third completion; current jobs finish |
| Production JS total / gzip | 649230 / 194991 bytes | Before dependency updates |
| Main route JS / gzip | 548758 / 156351 bytes | Existing large-chunk warning retained |
| Component navigation | 307 ms | Chrome/Vite fixture, not production/native startup |
| Library 20 / 200 cards | 78 / 232 ms | Interaction plus two rendered frames |
| 10000 selected paths | 467 ms | Synthetic queue state plus rendered output |
| Modal open/Escape | 31–78 ms | Ten cycles, focus return verified |

Decision: retain serial scans. A bounded pool improves cold profiling, but it needs integrated cooperative cancellation, native progress/order acceptance and disk-pressure scheduling before shipping. The pressure signal above is injected; no host disk was filled. Child RSS high-water marks (64–77 MiB for baseline scans) are cumulative and do not measure aggregate worker memory. The successful-profile cache already reduces repeated scan work; live forecasts and host inspection remain fresh.

Decision: retain eager YouTube forecasts and their existing JSON fields. Their measured cost is small relative to profiling. Keep the bundle warning visible; no measured runtime regression or justified product chunk/loading redesign was established. Dependency-batch comparisons and native acceptance are recorded separately.

Reproduce scan measurements with `tools/benchmark_scan.py`; run the explicit Rust cache benchmark from README. Run UI measurements with `node tests/performance-contract.mjs` from `desktop`. These generate isolated fixtures and do not authorize media-library access.
