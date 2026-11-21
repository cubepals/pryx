// SPDX-License-Identifier: FSL-1.1-ALv2
//! Copy complete vector groups (or an exact REP byte count), then copy the tail.

use super::{Plan, l1_data_cache};
#[cfg(not(miri))]
use super::{c_void, pryx_free_scan, pryx_scan_bytes};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CopyPlan(Plan);

impl CopyPlan {
    /// CPUID fast-string features in (ERMS, FSRM) order.
    pub fn string_features() -> (bool, bool) {
        #[cfg(all(target_arch = "x86_64", not(miri)))]
        {
            use std::arch::x86_64::{__cpuid, __cpuid_count};
            if __cpuid(0).eax >= 7 {
                let features = __cpuid_count(7, 0);
                return (features.ebx & (1 << 9) != 0, features.edx & (1 << 4) != 0);
            }
        }
        (false, false)
    }

    pub fn group_bytes(self) -> usize {
        self.0.width * self.0.unroll
    }

    fn rep() -> Self {
        Self(Plan { width: 1, unroll: 1 })
    }

    pub fn choices() -> Vec<Self> {
        let mut plans: Vec<_> = Plan::static_choices().into_iter().map(Self).collect();
        let (erms, fsrm) = Self::string_features();
        if erms || fsrm {
            plans.push(Self::rep());
        }
        plans
    }

    pub fn detected(length_hint: usize) -> Option<Self> {
        let (erms, fsrm) = Self::string_features();
        let (line, capacity) = l1_data_cache();
        if (fsrm && length_hint <= 128) || (erms && length_hint >= line) {
            return Some(Self::rep());
        }
        let choices = Plan::static_choices();
        let width = choices.last()?.width;
        // Keep the smaller body when source and destination both fit in L1D.
        let group_bytes =
            if capacity.is_some_and(|size| length_hint.saturating_mul(2) <= size) { line / 2 } else { line };
        choices.into_iter().rev().find(|p| p.width == width && width * p.unroll <= group_bytes.max(width)).map(Self)
    }

    pub fn slug(self) -> String {
        format!("copy-{}", self.0.slug())
    }
}

pub fn copy_scalar(dst: &mut [u8], src: &[u8]) {
    dst.copy_from_slice(src);
}

#[cfg(not(miri))]
type Entry = unsafe extern "C" fn(*mut u8, *const u8, usize);

#[cfg(not(miri))]
unsafe extern "C" {
    fn pryx_generate_copy(width: u32, count: u32) -> *mut c_void;
    fn pryx_copy_entry(handle: *mut c_void) -> Entry;
}

pub struct CopyKernel {
    plan: CopyPlan,
    #[cfg(not(miri))]
    owner: *mut c_void,
    #[cfg(not(miri))]
    entry: Entry,
}

impl CopyKernel {
    pub fn generate(length_hint: usize) -> Option<Self> {
        Self::for_plan(CopyPlan::detected(length_hint)?)
    }

    pub fn for_plan(plan: CopyPlan) -> Option<Self> {
        #[cfg(not(miri))]
        unsafe {
            let owner = pryx_generate_copy(plan.0.width as u32, plan.0.unroll as u32);
            if !owner.is_null() {
                return Some(Self { plan, owner, entry: pryx_copy_entry(owner) });
            }
        }
        let _ = plan;
        None
    }

    pub fn plan(&self) -> CopyPlan {
        self.plan
    }

    pub fn encoding(&self) -> Option<&[u8]> {
        #[cfg(not(miri))]
        {
            let mut size = 0;
            let data = unsafe { pryx_scan_bytes(self.owner, &mut size) };
            Some(unsafe { std::slice::from_raw_parts(data, size) })
        }
        #[cfg(miri)]
        None
    }

    pub fn copy(&self, dst: &mut [u8], src: &[u8]) {
        assert_eq!(dst.len(), src.len());
        #[cfg(miri)]
        let copied = 0;
        #[cfg(not(miri))]
        let copied = {
            let group = self.plan.group_bytes();
            let groups = src.len() / group;
            if groups > 0 {
                // Slice borrows are disjoint and contain this many whole groups.
                unsafe { (self.entry)(dst.as_mut_ptr(), src.as_ptr(), groups) };
            }
            groups * group
        };
        copy_scalar(&mut dst[copied..], &src[copied..]);
    }
}

impl Drop for CopyKernel {
    fn drop(&mut self) {
        #[cfg(not(miri))]
        unsafe {
            pryx_free_scan(self.owner);
        }
    }
}
