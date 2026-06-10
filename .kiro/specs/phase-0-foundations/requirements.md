# Requirements Document

## Introduction

ForgeOS Phase 0 ("Foundations") builds the deterministic, clean-room plumbing
that every later phase depends on. The deliverables are: the Cargo workspace
wiring for the first two crates, a CI gate (build + clippy + test), the
`forge-core` fail-fast domain types (`UnixNanos`, fixed-point `Price`/`Qty`,
`Side`, `Event`), and the `forge-data` hot path (the `*.forge` packed record,
a time-sorted writer, a zero-copy mmap reader, and a clean-room Rust converter
from cryptohftdata Parquet to `*.forge`).

The phase culminates in the **Phase 0 Gate**: round-trip a known data window
(Parquet -> `*.forge` -> read back) and prove event-count equality, timestamp
monotonicity, and a content checksum match, while the read loop performs zero
allocation. The gate is wired into CI.

Phase 0 is strictly foundational. Book reconstruction, the simulation/matching
engine, fills, P&L, metrics, the null-edge harness, and parameter sweeps are
all out of scope and belong to later phases. The configurable two-clock
feed-latency model is also explicitly deferred to Phase 1; Phase 0 writes
`local_ts = exch_ts` (feed latency of zero).

## Glossary

- **ForgeOS**: The clean-room, Rust-native, deterministic research/backtest engine this spec contributes to.
- **forge_core**: The Rust crate providing fail-fast domain types (`UnixNanos`, `Price`, `Qty`, `Side`, `Event`, errors).
- **forge_data**: The Rust crate providing the `*.forge` packed record, writer, zero-copy mmap reader, and converter binary (the hot path).
- **Workspace**: The Cargo workspace defined by the repository root `Cargo.toml`.
- **CI_Pipeline**: The automated continuous-integration system (GitHub Actions or equivalent) that runs build, lint, and test gates on each change.
- **UnixNanos**: A `u64` value representing a Unix timestamp in integer nanoseconds.
- **Price**: A fixed-point monetary value stored as `i64` at a scale of 1e-8 (one unit = 0.00000001).
- **Qty**: A fixed-point quantity value stored as `i64` at a scale of 1e-8.
- **Fixed_Scale**: The constant integer scale factor 100,000,000 (1e-8 represented as `i64`).
- **Side**: An enumeration with exactly the values `Bid` and `Ask`.
- **Event**: A normalized market-data event enum with variants `Trade`, `BookDelta`, `BookSnapshot`, and `Quote`.
- **EventRecord**: The `#[repr(C)]`, plain-old-data, fixed-size struct that is the on-disk and in-memory record of the `*.forge` format.
- **forge_file**: A binary file with the `*.forge` extension containing a contiguous, time-sorted sequence of `EventRecord` values.
- **exch_ts**: The `UnixNanos` timestamp at which an event occurred at the venue.
- **local_ts**: The `UnixNanos` timestamp at which the strategy could first observe an event (`exch_ts + feed_latency`); in Phase 0, `feed_latency = 0` so `local_ts = exch_ts`.
- **Writer**: The `forge_data` component that serializes normalized events into a time-sorted `forge_file`.
- **Reader**: The `forge_data` component that mmaps a `forge_file` and exposes its bytes as `&[EventRecord]` with zero copy and zero decode.
- **Converter**: The `forge_data` Rust binary target that transforms cryptohftdata Parquet inputs into a single time-sorted `forge_file`.
- **Checksum**: A deterministic content hash computed over a normalized, ordered representation of an event sequence, used to verify round-trip fidelity.
- **Round_Trip**: The pipeline Parquet -> `forge_file` (write) -> `&[EventRecord]` (read) used to validate fidelity.
- **Clean_Room**: The discipline of re-deriving logic from specifications and behavior without copying source code from external repositories or legacy code.
- **Phase_0_Gate**: The set of acceptance criteria that validate the round-trip and the zero-allocation read loop, wired into the CI_Pipeline.

## Requirements

### Requirement 1: Cargo Workspace Activation

**User Story:** As a ForgeOS developer, I want the `forge-core` and `forge-data` crates enabled in the workspace, so that Phase 0 code compiles as part of the standard build.

#### Acceptance Criteria

1. THE Workspace SHALL include `crates/forge-core` as an active workspace member.
2. THE Workspace SHALL include `crates/forge-data` as an active workspace member.
3. THE Workspace SHALL retain the release profile settings `panic = "abort"` and `lto = "thin"`.
4. WHEN `cargo build --release` is executed at the repository root, THE Workspace SHALL compile all active members without errors.
5. THE Workspace SHALL exclude `crates/forge-book`, `crates/forge-sim`, `crates/forge-strategy`, `crates/forge-metrics`, and `crates/forge-sweep` from the active member list during Phase 0.

### Requirement 2: Continuous Integration Gates

**User Story:** As a ForgeOS developer, I want CI to enforce build, lint, and test gates, so that every change preserves clean-room and deterministic discipline.

#### Acceptance Criteria

1. WHEN a change is pushed to the repository, THE CI_Pipeline SHALL run a release build of the Workspace.
2. WHEN a change is pushed to the repository, THE CI_Pipeline SHALL run `cargo clippy` across the Workspace.
3. WHEN a change is pushed to the repository, THE CI_Pipeline SHALL run `cargo test` across the Workspace.
4. IF the build step fails, THEN THE CI_Pipeline SHALL report a failing status and block the change from being marked as passing.
5. IF the clippy step reports any warning or error, THEN THE CI_Pipeline SHALL report a failing status.
6. IF any test fails, THEN THE CI_Pipeline SHALL report a failing status.

### Requirement 3: UnixNanos Timestamp Type

**User Story:** As a ForgeOS developer, I want a nanosecond timestamp type with validation helpers, so that all event ordering is unambiguous and fail-fast.

#### Acceptance Criteria

1. THE forge_core SHALL provide a `UnixNanos` type backed by a `u64` representing integer nanoseconds since the Unix epoch.
2. THE forge_core SHALL provide a helper that reports whether a sequence of `UnixNanos` values is monotonically non-decreasing.
3. WHEN two `UnixNanos` values are compared, THE forge_core SHALL order them by their underlying `u64` value.
4. WHERE a timestamp is supplied from a signed source, THE forge_core SHALL reject negative timestamp values as an error before constructing a `UnixNanos`.

### Requirement 4: Fixed-Point Price and Qty with Checked Arithmetic

**User Story:** As a ForgeOS developer, I want price and quantity as fixed-point integers with checked arithmetic, so that value and P&L paths never use floating point and never silently overflow.

#### Acceptance Criteria

1. THE forge_core SHALL represent `Price` as an `i64` at Fixed_Scale of 1e-8.
2. THE forge_core SHALL represent `Qty` as an `i64` at Fixed_Scale of 1e-8.
3. THE forge_core SHALL exclude floating-point types from the stored representation of `Price` and `Qty`.
4. WHEN `Price` or `Qty` values are added, subtracted, or multiplied, THE forge_core SHALL use checked arithmetic that detects integer overflow.
5. IF an arithmetic operation on `Price` or `Qty` overflows the `i64` range, THEN THE forge_core SHALL return an error in release builds and panic in debug and test builds.
6. IF a floating-point value converted to `Price` or `Qty` is NaN or infinite, THEN THE forge_core SHALL reject the conversion as an error.
7. IF a floating-point value converted to `Price` or `Qty` falls outside the representable `i64` fixed-point range, THEN THE forge_core SHALL reject the conversion as an error.
8. WHERE a floating-point value is converted to `Price` or `Qty`, THE forge_core SHALL scale the value by Fixed_Scale and round to the nearest representable integer.

### Requirement 5: Side and Event Domain Types

**User Story:** As a ForgeOS developer, I want canonical `Side` and `Event` types, so that normalized market data has one well-defined in-memory representation.

#### Acceptance Criteria

1. THE forge_core SHALL provide a `Side` enumeration with exactly the values `Bid` and `Ask`.
2. THE forge_core SHALL provide an `Event` enumeration with exactly the variants `Trade`, `BookDelta`, `BookSnapshot`, and `Quote`.
3. THE forge_core SHALL associate each `Event` variant with an `exch_ts` of type `UnixNanos` and a `local_ts` of type `UnixNanos`.
4. THE forge_core SHALL represent price and quantity fields carried by any `Event` variant using `Price` and `Qty` fixed-point types.

### Requirement 6: Fail-Fast Validation at Ingest

**User Story:** As a ForgeOS developer, I want bad data rejected at ingest, so that corrupt input never silently contaminates downstream results.

#### Acceptance Criteria

1. IF an ingested price or quantity originates from a NaN or infinite floating-point value, THEN THE forge_data SHALL reject the record as a hard error at ingest.
2. IF an ingested timestamp is negative, THEN THE forge_data SHALL reject the record as a hard error at ingest.
3. IF an ingested event has a `local_ts` that is less than the `local_ts` of the previously ingested event, THEN THE forge_data SHALL reject the stream as a non-monotonic hard error at ingest.
4. IF an arithmetic operation during ingest overflows the fixed-point range, THEN THE forge_data SHALL reject the record as a hard error at ingest.
5. WHILE running in debug or test builds, THE forge_data SHALL convert the conditions in acceptance criteria 1 through 4 into panics.

### Requirement 7: Packed `*.forge` Record Format

**User Story:** As a ForgeOS developer, I want a fixed-size POD record layout, so that the hot path can reinterpret raw bytes as records with zero decoding.

#### Acceptance Criteria

1. THE forge_data SHALL define `EventRecord` as a `#[repr(C)]` plain-old-data struct.
2. THE EventRecord SHALL occupy exactly 32 bytes.
3. THE EventRecord SHALL contain the fields `exch_ts: u64`, `local_ts: u64`, `kind: u8`, `side: u8`, `price: i64`, and `qty: i64`.
4. THE EventRecord SHALL include explicit flag and padding fields sufficient to reach the fixed 32-byte size with no implicit compiler padding.
5. THE EventRecord SHALL be safely reinterpretable from a byte slice via a zero-copy casting facility such as bytemuck or zerocopy.
6. THE forge_data SHALL store `price` and `qty` fields of `EventRecord` as fixed-point `i64` values at Fixed_Scale.

### Requirement 8: Time-Sorted Writer

**User Story:** As a ForgeOS developer, I want a writer that emits a time-sorted `*.forge` file, so that replay can scan events in chronological order.

#### Acceptance Criteria

1. WHEN the Writer serializes a sequence of normalized events, THE Writer SHALL produce a forge_file containing one EventRecord per event.
2. THE Writer SHALL order the EventRecord values in the forge_file by non-decreasing `local_ts`.
3. IF the input event sequence is not ordered by non-decreasing `local_ts`, THEN THE Writer SHALL sort the events into non-decreasing `local_ts` order before serialization.
4. WHEN the Writer serializes the same input event sequence more than once, THE Writer SHALL produce byte-identical forge_file output.

### Requirement 9: Zero-Copy mmap Reader

**User Story:** As a ForgeOS developer, I want a memory-mapped reader that exposes records as a slice, so that the replay read loop allocates nothing and decodes nothing.

#### Acceptance Criteria

1. WHEN the Reader opens a forge_file, THE Reader SHALL memory-map the file and expose its contents as `&[EventRecord]`.
2. WHILE iterating the records of an opened forge_file, THE Reader SHALL perform zero heap allocation in the read loop.
3. WHILE iterating the records of an opened forge_file, THE Reader SHALL perform zero per-record decoding or deserialization.
4. THE Reader SHALL expose the records as a contiguous sequential scan in stored order.
5. IF the byte length of a forge_file is not an exact multiple of the EventRecord size, THEN THE Reader SHALL reject the file as a hard error.

### Requirement 10: Clean-Room Parquet-to-`*.forge` Converter

**User Story:** As a ForgeOS developer, I want a Rust converter binary from cryptohftdata Parquet to `*.forge`, so that cold interchange data becomes a single normalized hot-path stream.

#### Acceptance Criteria

1. THE forge_data SHALL provide a Rust binary target that converts cryptohftdata Parquet inputs into a forge_file.
2. WHEN the Converter is given trade, book-delta, and quote Parquet inputs for a window, THE Converter SHALL merge them into a single stream ordered by non-decreasing `local_ts`.
3. THE Converter SHALL normalize each input row into an EventRecord using the forge_core domain types.
4. THE Converter SHALL be implemented clean-room without copying source code from `tools/chd-to-parquet.py`, external repositories, or legacy code.
5. WHEN the Converter processes the same Parquet input more than once, THE Converter SHALL produce byte-identical forge_file output.
6. IF a Parquet input row fails fail-fast validation, THEN THE Converter SHALL reject the row according to Requirement 6.

### Requirement 11: Phase 0 local_ts Assignment

**User Story:** As a ForgeOS developer, I want Phase 0 to carry both clocks but set feed latency to zero, so that the two-clock model is structurally ready without committing to latency assignment yet.

#### Acceptance Criteria

1. THE EventRecord SHALL carry both an `exch_ts` field and a `local_ts` field.
2. WHEN the Converter or Writer assigns `local_ts`, THE forge_data SHALL set `local_ts` equal to `exch_ts` (a feed latency of zero).
3. THE forge_data SHALL exclude configurable feed-latency assignment from Phase 0 scope, deferring the two-clock latency model to Phase 1.

### Requirement 12: Phase 0 Round-Trip Gate

**User Story:** As a ForgeOS developer, I want a CI-enforced round-trip gate, so that the data pipeline provably preserves event count, ordering, and content.

#### Acceptance Criteria

1. WHEN a known data window is processed through the Round_Trip, THE forge_data SHALL produce a read-back event count equal to the source event count.
2. WHEN a known data window is read back from a forge_file, THE forge_data SHALL confirm that every record's `local_ts` is greater than or equal to the previous record's `local_ts`.
3. WHEN a known data window is processed through the Round_Trip, THE forge_data SHALL compute a Checksum over the read-back events that equals the Checksum computed over the source events.
4. WHILE executing the read-back loop of the Round_Trip gate, THE forge_data SHALL perform zero heap allocation.
5. THE CI_Pipeline SHALL execute the Phase_0_Gate as a test that must pass for the pipeline to report a passing status.
6. IF any of the event-count, monotonicity, Checksum, or zero-allocation checks fails, THEN THE Phase_0_Gate SHALL report a failing status.

### Requirement 13: Determinism

**User Story:** As a ForgeOS developer, I want deterministic outputs, so that identical inputs always yield identical bytes and hashes.

#### Acceptance Criteria

1. WHEN the same source input is converted and written more than once, THE forge_data SHALL produce forge_file outputs that are byte-identical.
2. WHEN the same forge_file is read more than once, THE Reader SHALL yield an identical sequence of EventRecord values.
3. WHEN a Checksum is computed over the same event sequence more than once, THE forge_data SHALL produce an identical Checksum value.
4. THE forge_data SHALL exclude wall-clock time, unseeded randomness, and ordering-dependent iteration from the conversion and write paths.

### Requirement 14: Clean-Room and Commit Discipline

**User Story:** As a ForgeOS developer, I want clean-room re-derivation and small tested commits, so that the executing core stays trustworthy and auditable.

#### Acceptance Criteria

1. THE forge_core and forge_data SHALL be implemented without copying source code from external repositories including nautilus_trader and hftbacktest.
2. THE forge_core and forge_data SHALL be implemented without copying source code from `docs/legacy` or other legacy code.
3. WHERE behavior is derived from a reference, THE developer SHALL re-derive the logic into ForgeOS primitives and accompany it with a test.
4. WHEN a change touches the conversion, write, read, or validation paths, THE change SHALL include a test that fails if that change's failure mode is reintroduced.