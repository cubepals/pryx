// SPDX-License-Identifier: FSL-1.1-ALv2
//! Persistent shell state contains process handles and output queues, without HTTP dependencies.
use crate::process::{Event, Process};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};
use tokio::sync::{Mutex as AsyncMutex, mpsc};
#[derive(Default)]
pub struct Shells(pub(crate) Mutex<HashMap<String, Arc<AsyncMutex<Shell>>>>);
pub(crate) struct Shell {
    pub(crate) process: Arc<Process>,
    pub(crate) replay: VecDeque<Event>,
    pub(crate) live: mpsc::Receiver<Event>,
}
