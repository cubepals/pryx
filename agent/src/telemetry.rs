// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Activity is atomic memory on the hot path and a bounded checkpoint once per second. The
//! node's existing heartbeat carries it to lifecycle upkeep; requests never call control.

use crate::state::Activity;
use std::sync::{
    Arc,
    atomic::{AtomicI64, AtomicU32, Ordering},
};

#[derive(Default)]
pub struct Telemetry {
    at: AtomicI64,
    streams: AtomicU32,
}

impl Telemetry {
    pub fn touch(&self) {
        self.at.store(now(), Ordering::Relaxed);
    }
    pub fn active(self: &Arc<Self>) -> Active {
        self.touch();
        self.streams.fetch_add(1, Ordering::Relaxed);
        Active(self.clone())
    }
    pub fn snapshot(&self) -> Activity {
        let streams = self.streams.load(Ordering::Relaxed);
        Activity { at: if streams > 0 { now() } else { self.at.load(Ordering::Relaxed) }, streams }
    }
}

pub struct Active(Arc<Telemetry>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.touch();
        self.0.streams.fetch_sub(1, Ordering::Relaxed);
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}
