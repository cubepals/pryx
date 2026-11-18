// SPDX-License-Identifier: FSL-1.1-ALv2
//! The address ranges named by the host network protections. Data only; each consumer
//! enforces its own policy.

/// Private IPv4: RFC 1918's ranges, and RFC 6598's shared space, where WireGuard and Tailscale
/// overlays live (as infer.rs counts them).
pub(crate) const PRIVATE_V4: &str = "10.0.0.0/8, 100.64.0.0/10, 172.16.0.0/12, 192.168.0.0/16";
/// Private IPv6: unique local addresses (RFC 4193) and link-local ones (RFC 4291).
pub(crate) const PRIVATE_V6: &str = "fc00::/7, fe80::/10";
/// IPv4 link-local (RFC 3927), where 169.254.169.254 serves instance metadata on most clouds.
pub(crate) const LINK_LOCAL_V4: &str = "169.254.0.0/16";
