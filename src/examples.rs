// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Generic ports for test example specs. The daemon takes every port from the caller's spec.

/// The example HTTP application's listener.
pub const HTTP_PORT: u16 = 8080;
/// A second TCP listener for application administration.
pub const ADMIN_PORT: u16 = 9090;
/// The example application's UDP listener.
pub const UDP_PORT: u16 = 9091;
