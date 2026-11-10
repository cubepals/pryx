// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Guarantees describe measured enforcement, independently of a tier's name. Their names are
//! also feature names; consumers must treat unknown or unannounced guarantees as false.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum Guarantee {
    SeparateUsers,
    MemoryLimit,
    CpuLimit,
    PrivateNetwork,
    HostFirewall,
    OwnKernel,
    MemorySleep,
    BrowserSandbox,
    PrivatePids,
}
impl Guarantee {
    pub(crate) const ALL: [Self; 9] = [
        Self::SeparateUsers,
        Self::MemoryLimit,
        Self::CpuLimit,
        Self::PrivateNetwork,
        Self::HostFirewall,
        Self::OwnKernel,
        Self::MemorySleep,
        Self::BrowserSandbox,
        Self::PrivatePids,
    ];
    pub(crate) fn feature(self) -> String {
        serde_json::to_value(self).expect("a guarantee serializes as a string").as_str().unwrap().to_owned()
    }
}
