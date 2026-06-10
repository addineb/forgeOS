//! Phase 0 GATE (docs/roadmap.md): a known window round-trips
//! events -> `*.forge` -> read-back with byte-exact fidelity, and the read loop
//! allocates zero bytes per event.
//!
//! Requirement 8.6 permits a deterministically generated synthetic window so
//! the gate runs in CI without the Hetzner parquet feed. The checks are:
//!   1. read-back count == source count
//!   2. local_ts is non-decreasing end to end
//!   3. checksum over read-back bytes == writer's recorded checksum
//!   4. the scan loop performs zero heap allocations

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::BufWriter;
use std::sync::atomic::{AtomicUsize, Ordering};

use forge_data::{synthetic, ForgeReader, ForgeWriter};

/// Allocation-counting global allocator. It delegates to the system allocator
/// and bumps a counter on every alloc/realloc, so a tight read loop can assert
/// it allocated nothing.
struct CountingAlloc;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

#[test]
fn roundtrip_gate() {
    const N: usize = 50_000;

    // --- write a synthetic window (allocations here are irrelevant) ---
    let events = synthetic::generate(0xF0F0_1234, N);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("gate.forge");

    let file = std::fs::File::create(&path).expect("create");
    let mut writer = ForgeWriter::new(BufWriter::new(file));
    for ev in &events {
        writer.write_event(ev).expect("write_event");
    }
    let meta = writer.finish().expect("finish");

    assert_eq!(meta.count as usize, N, "writer counted wrong number of events");
    assert_eq!(
        meta.bytes,
        meta.count * forge_data::RECORD_SIZE as u64,
        "no framing drift"
    );

    // --- read back via mmap ---
    let reader = ForgeReader::open(&path).expect("open");
    assert_eq!(reader.len(), N, "read-back count must equal source count");

    // --- measure allocations strictly around the scan loop ---
    let records = reader.records();
    let before = ALLOCS.load(Ordering::Relaxed);
    let mut acc: u64 = 0;
    let mut last_lt: u64 = 0;
    let mut monotonic = true;
    for rec in records {
        if rec.local_ts < last_lt {
            monotonic = false;
        }
        last_lt = rec.local_ts;
        acc = acc
            .wrapping_add(rec.local_ts)
            .wrapping_add(rec.exch_ts)
            .wrapping_add(rec.price as u64)
            .wrapping_add(rec.qty as u64);
    }
    let after = ALLOCS.load(Ordering::Relaxed);
    std::hint::black_box(acc);

    assert_eq!(after - before, 0, "read loop allocated {} time(s)", after - before);
    assert!(monotonic, "local_ts must be non-decreasing end to end");

    // --- checksum of read-back bytes matches the writer's stamp ---
    assert_eq!(
        reader.checksum(),
        meta.checksum,
        "read-back checksum must match the writer stamp"
    );

    // --- full-fidelity spot check: decode equals the source events ---
    for (rec, ev) in records.iter().zip(events.iter()) {
        assert_eq!(rec.to_event().expect("decode"), *ev);
    }
}