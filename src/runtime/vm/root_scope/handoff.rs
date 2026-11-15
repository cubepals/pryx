// SPDX-License-Identifier: FSL-1.1-ALv2
//! Receive bounded authority from the verified scope program, with close-on-exec ownership
//! even on malformed or truncated messages. No namespace transition occurs in the daemon.
use rustix::net::{self, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io,
    mem::MaybeUninit,
    os::fd::{AsFd, OwnedFd},
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "camelCase")]
pub(super) enum Role {
    Namespace,
    PacketTap,
    PacketConfiguration,
    PacketIdentity,
    Http,
    Tls,
    DnsTcp,
    DnsUdp,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Advertisement {
    #[serde(default)]
    pub host_firewall: bool,
    #[serde(default)]
    pub bridge_host_firewall: bool,
    #[serde(default)]
    pub bridge_descriptors: Vec<Role>,
    #[serde(default)]
    pub bridge_agent_argument: String,
    #[serde(default)]
    pub bridge_packet_argument: String,
    #[serde(default)]
    pub packet_runtime_argument: String,
    #[serde(default)]
    pub packet_runtime_descriptors: Vec<Role>,
    pub descriptors: Vec<Role>,
    pub max_descriptors: usize,
    pub max_metadata_bytes: usize,
    pub version: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    #[serde(default)]
    host_firewall: bool,
    descriptors: Vec<Role>,
    profile: Value,
    tap: String,
    version: u64,
    #[serde(default)]
    packet_tap: String,
    #[serde(default)]
    ipv4_exclusions: usize,
    #[serde(default)]
    packet_paths: Option<PacketPaths>,
    #[serde(default)]
    packet_profile: Value,
}

pub(super) struct Grant {
    pub namespace: OwnedFd,
    pub http: OwnedFd,
    pub tls: OwnedFd,
    pub dns_tcp: OwnedFd,
    pub dns_udp: OwnedFd,
    pub profile: Value,
    pub tap: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PacketPaths {
    pub launcher: std::path::PathBuf,
    pub program: std::path::PathBuf,
    pub directory: std::path::PathBuf,
    pub configuration: std::path::PathBuf,
    pub lease: std::path::PathBuf,
    pub parent: u32,
}

pub(super) struct BridgeGrant {
    pub configuration: Option<OwnedFd>,
    pub paths: Option<PacketPaths>,
    pub packet_profile: Value,
    pub namespace: OwnedFd,
    pub packet: OwnedFd,
    pub profile: Value,
    pub tap: String,
    pub packet_tap: String,
}

pub(super) fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Root scope descriptor grant differs from its executable metadata")
}

impl Advertisement {
    pub(super) fn bridge(mut self) -> io::Result<Self> {
        if !self.bridge_host_firewall {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "Root scope omits its public bridge parser"));
        }
        for argument in [&self.bridge_agent_argument, &self.bridge_packet_argument] {
            if argument.strip_prefix("--").is_none_or(|name| {
                name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'-')
            }) {
                return Err(invalid());
            }
        }
        if self.bridge_agent_argument == self.bridge_packet_argument {
            return Err(invalid());
        }
        self.descriptors = std::mem::take(&mut self.bridge_descriptors);
        self.validate_roles(&[Role::Namespace, Role::PacketTap])?;
        Ok(self)
    }
    pub(super) fn packet(mut self) -> io::Result<Self> {
        if self
            .packet_runtime_argument
            .strip_prefix("--")
            .is_none_or(|name| name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'-'))
            || [&self.bridge_agent_argument, &self.bridge_packet_argument].contains(&&self.packet_runtime_argument)
        {
            return Err(invalid());
        }
        self.descriptors = std::mem::take(&mut self.packet_runtime_descriptors);
        self.validate_roles(&[Role::Namespace, Role::PacketTap, Role::PacketConfiguration])?;
        Ok(self)
    }
    pub(super) fn validate(&self) -> io::Result<()> {
        self.validate_roles(&[Role::Namespace, Role::Http, Role::Tls, Role::DnsTcp, Role::DnsUdp])
    }
    fn validate_roles(&self, expected: &[Role]) -> io::Result<()> {
        if !self.host_firewall {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Root TAP helper does not advertise its namespace firewall",
            ));
        }
        // This is the receiver's allocation ceiling, not the producer's protocol limit.
        const MAX_RECEIVED_FILES: usize = 32;
        if self.max_descriptors == 0
            || self.max_descriptors > MAX_RECEIVED_FILES
            || self.descriptors.len() > self.max_descriptors
            || self.max_metadata_bytes == 0
            || self.max_metadata_bytes > pryx_agent::contract::MAX_REQUEST_BYTES
        {
            return Err(invalid());
        }
        let mut roles = self.descriptors.clone();
        roles.sort();
        if roles != expected {
            return Err(invalid());
        }
        Ok(())
    }
}

pub(super) fn receive(socket: impl AsFd, advertisement: &Advertisement) -> io::Result<Grant> {
    let (metadata, mut files) = receive_files(socket, advertisement)?;
    Ok(Grant {
        namespace: files.remove(&Role::Namespace).ok_or_else(invalid)?,
        http: files.remove(&Role::Http).ok_or_else(invalid)?,
        tls: files.remove(&Role::Tls).ok_or_else(invalid)?,
        dns_tcp: files.remove(&Role::DnsTcp).ok_or_else(invalid)?,
        dns_udp: files.remove(&Role::DnsUdp).ok_or_else(invalid)?,
        profile: metadata.profile,
        tap: metadata.tap,
    })
}

pub(super) fn receive_bridge(socket: impl AsFd, advertisement: &Advertisement) -> io::Result<BridgeGrant> {
    let (metadata, mut files) = receive_files(socket, advertisement)?;
    if metadata.ipv4_exclusions == 0
        || metadata.packet_tap.is_empty()
        || metadata.packet_tap.len() >= libc::IF_NAMESIZE
        || metadata.packet_tap.contains('/')
    {
        return Err(invalid());
    }
    Ok(BridgeGrant {
        configuration: files.remove(&Role::PacketConfiguration),
        paths: metadata.packet_paths,
        packet_profile: metadata.packet_profile,
        namespace: files.remove(&Role::Namespace).ok_or_else(invalid)?,
        packet: files.remove(&Role::PacketTap).ok_or_else(invalid)?,
        profile: metadata.profile,
        tap: metadata.tap,
        packet_tap: metadata.packet_tap,
    })
}

pub(super) fn receive_identity(socket: impl AsFd, advertisement: &Advertisement) -> io::Result<OwnedFd> {
    let mut metadata = advertisement.clone();
    metadata.descriptors = vec![Role::PacketIdentity];
    let (_, mut files) = receive_files(socket, &metadata)?;
    files.remove(&Role::PacketIdentity).ok_or_else(invalid)
}

fn receive_files(socket: impl AsFd, advertisement: &Advertisement) -> io::Result<(Metadata, BTreeMap<Role, OwnedFd>)> {
    let mut payload = vec![0; advertisement.max_metadata_bytes];
    let mut space = vec![MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(advertisement.max_descriptors))];
    let mut control = RecvAncillaryBuffer::new(&mut space);
    let message =
        net::recvmsg(socket, &mut [io::IoSliceMut::new(&mut payload)], &mut control, RecvFlags::CMSG_CLOEXEC)?;
    // Rustix owns and closes every remaining SCM_RIGHTS entry when the buffer is dropped.
    if message.flags.intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC) || message.bytes == 0 {
        return Err(invalid());
    }
    let metadata: Metadata = serde_json::from_slice(&payload[..message.bytes]).map_err(|_| invalid())?;
    if !metadata.host_firewall
        || metadata.version != advertisement.version
        || metadata.descriptors != advertisement.descriptors
    {
        return Err(invalid());
    }
    let mut files = Vec::new();
    for message in control.drain() {
        match message {
            RecvAncillaryMessage::ScmRights(rights) => files.extend(rights),
            _ => return Err(invalid()),
        }
    }
    if files.len() != metadata.descriptors.len() {
        return Err(invalid());
    }
    let files = metadata.descriptors.iter().copied().zip(files).collect();
    Ok((metadata, files))
}
