// SPDX-License-Identifier: FSL-1.1-ALv2
//! Compile the actual serving entry under a separate opt-in filename so the
//! staged stock launcher never changes during the experiment.
#[path = "../../../agent/src/main.rs"]
mod serving;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    serving::run_lab_service()
}
