// SPDX-License-Identifier: FSL-1.1-ALv2
//! Native tool lifetime storage is opaque to the process supervisor. Tool controllers own
//! their typed state; children receive only non-secret runtime paths, never bootstrap secrets.
use std::{
    any::Any,
    collections::BTreeMap,
    sync::{Arc, Mutex, RwLock},
};

#[derive(Default)]
pub(crate) struct ToolSlots {
    pub(crate) state: Mutex<Option<Arc<dyn Any + Send + Sync>>>,
    pub(crate) environment: RwLock<BTreeMap<String, String>>,
}
