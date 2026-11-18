// SPDX-License-Identifier: FSL-1.1-ALv2
//! The address ranges pryx's host protections name, as nftables writes a set: the host firewall
//! (`isolation.rs`) and the process tier's kernel programs (`runtime/process/network.rs`) keep
//! workloads off the same ones. Not where they are enforced.

/// Private IPv4: RFC 1918's ranges, and RFC 6598's shared space, where WireGuard and Tailscale
/// overlays live (as infer.rs counts them).
pub(crate) const PRIVATE_V4: &str = "10.0.0.0/8, 100.64.0.0/10, 172.16.0.0/12, 192.168.0.0/16";
/// Private IPv6: unique local addresses (RFC 4193) and link-local ones (RFC 4291).
pub(crate) const PRIVATE_V6: &str = "fc00::/7, fe80::/10";
/// IPv4 link-local (RFC 3927), where 169.254.169.254 serves instance metadata on most clouds.
pub(crate) const LINK_LOCAL_V4: &str = "169.254.0.0/16";
/// AWS's instance metadata service over IPv6 (Amazon EC2 User Guide, "Instance metadata service
/// endpoints"). It sits in the unique local range, so it is refused on every port as IPv4's
/// link-local metadata is, while the rest of fc00::/7, AWS's resolver fd00:ec2::253 included,
/// stays an ordinary private address.
pub(crate) const METADATA_V6: &str = "fd00:ec2::254";
