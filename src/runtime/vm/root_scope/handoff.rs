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
    Http,
    Tls,
    DnsTcp,
    DnsUdp,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Advertisement {
    pub descriptors: Vec<Role>,
    pub max_descriptors: usize,
    pub max_metadata_bytes: usize,
    pub version: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    descriptors: Vec<Role>,
    profile: Value,
    tap: String,
    version: u64,
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

pub(super) fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Root scope descriptor grant differs from its executable metadata")
}

impl Advertisement {
    pub(super) fn validate(&self) -> io::Result<()> {
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
        if roles != [Role::Namespace, Role::Http, Role::Tls, Role::DnsTcp, Role::DnsUdp] {
            return Err(invalid());
        }
        Ok(())
    }
}

pub(super) fn receive(socket: impl AsFd, advertisement: &Advertisement) -> io::Result<Grant> {
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
    if metadata.version != advertisement.version || metadata.descriptors != advertisement.descriptors {
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
    let mut files: BTreeMap<_, _> = metadata.descriptors.into_iter().zip(files).collect();
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
