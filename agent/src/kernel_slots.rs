// SPDX-License-Identifier: FSL-1.1-ALv2
//! Kernel state belongs to the process supervisor's lifetime. This storage knows no process
//! or kernel implementation, so the supervisor does not depend on its own higher-level client.
use std::{
    any::Any,
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Default)]
pub(crate) struct KernelSlots(pub(crate) Mutex<HashMap<(String, String), Arc<dyn Any + Send + Sync>>>);
