// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Log persistence, bounded rotation and slow-subscriber signaling use real descriptors.
use base64::{Engine, engine::general_purpose::STANDARD};
use pryx_agent::{
    contract::CAPTURE_BYTES,
    logbook::{Frame, Logbook},
    paths::DataDir,
};

#[test]
fn logs_survive_restart_and_internal_state_stays_out_of_user_lists() {
    let dir = tempfile::tempdir().unwrap();
    let book = Logbook::open(dir.path()).unwrap();
    book.record("stdout", &STANDARD.encode(b"first\n")).unwrap();
    drop(book);
    let book = Logbook::open(dir.path()).unwrap();
    let (frames, mut live) = book.subscribe();
    let Frame::Replay { chunks } = &frames[0] else { panic!("replay") };
    assert_eq!(STANDARD.decode(&chunks[0].bytes).unwrap(), b"first\n");
    book.record("stderr", &STANDARD.encode(b"second\n")).unwrap();
    assert!(matches!(live.try_recv().unwrap(), Frame::Chunk { .. }));
    assert!(DataDir::open(dir.path()).unwrap().list(".").unwrap().is_empty());
}

#[test]
fn rotation_bounds_memory_and_disk_and_slow_readers_observe_a_gap() {
    let dir = tempfile::tempdir().unwrap();
    let book = Logbook::open(dir.path()).unwrap();
    let (_, mut slow) = book.subscribe();
    let bytes = STANDARD.encode(vec![b'x'; 8192]);
    for _ in 0..600 {
        book.record("stdout", &bytes).unwrap();
    }
    assert!(matches!(slow.try_recv(), Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_))));
    drop(book);
    let restored = Logbook::open(dir.path()).unwrap();
    let (frames, _) = restored.subscribe();
    let Frame::Replay { chunks } = &frames[0] else { panic!("replay") };
    assert!(!chunks.is_empty());
    assert!(chunks.iter().map(|c| c.bytes.len()).sum::<usize>() <= CAPTURE_BYTES);
    let meta = std::fs::metadata(dir.path().join(".pryx-agent/entrypoint-log.ndjson")).unwrap();
    assert!(meta.len() <= (CAPTURE_BYTES * 4) as u64);
}
