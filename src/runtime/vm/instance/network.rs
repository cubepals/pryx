// SPDX-License-Identifier: FSL-1.1-ALv2
//! Select only the host-owned VM network path. Ordinary users retain their confined packet
//! helper; root hostname policy uses its leased TAP namespace and existing gateways. Root
//! unrestricted forwarding stays a named refusal until its separate host integration exists.
use super::*;
use crate::protocol::EgressPolicy;
use pryx_agent::{vm_boot, vm_monitor};
use serde_json::Value;
use std::process::ExitStatus;
use tokio::time::Instant;

pub(super) enum Network {
    User(Box<super::super::network::Network>),
    Root { scope: Box<super::super::root_scope::Scope>, profile: Value },
}

impl Network {
    pub(super) async fn root(
        settings: &Settings,
        bundle: &Bundle,
        boots: &Path,
        lease: &Path,
        policy: EgressPolicy,
        deadline: Instant,
    ) -> Result<Self, RuntimeError> {
        if !bundle.bootstrap().metadata.vm_static_network || !bundle.bootstrap().metadata.vm_monitor_namespace {
            return Err(error("Root TAP requires advertised static guest and leased monitor namespace support"));
        }
        let mut profile =
            super::super::packet_profile::load(&bundle.root().join("bin/network-proxy"), deadline).await?;
        let scope = super::super::root_scope::Scope::start(
            super::super::root_scope::Paths {
                program: &bundle.root().join("bin/root-scope"),
                plugins: &bundle.root().join("bin/root-cni"),
                boots,
                lease,
                owner: settings.monitor_owner.ok_or_else(|| error("Missing fixed root monitor identities"))?,
            },
            policy,
            settings.ingress_connections,
            deadline,
        )
        .await?;
        let root_profile = scope.profile().as_object().ok_or_else(|| error("Missing root TAP profile"))?;
        let services = profile.as_object_mut().ok_or_else(|| error("Missing packet service metadata"))?;
        services.extend(root_profile.iter().map(|(key, value)| (key.clone(), value.clone())));
        Ok(Self::Root { scope: Box::new(scope), profile })
    }
    pub(super) fn guest(&self) -> Result<vm_boot::Network, RuntimeError> {
        match self {
            Self::User(network) => network.guest(),
            Self::Root { scope, profile } => {
                let (http, tls, dns) = scope.ports();
                let ports = crate::runtime::egress::listeners::Ports { http, tls, dns };
                let mut guest = super::super::packet_profile::guest(profile, &ports, false)?;
                guest.vsock_port = None;
                guest.static_ipv4 = Some(vm_boot::StaticNetwork {
                    address: profile["guestCidr"]
                        .as_str()
                        .ok_or_else(|| error("Missing static guest prefix"))?
                        .parse()
                        .map_err(error)?,
                    gateway: guest.policy.as_ref().ok_or_else(|| error("Missing root guest policy"))?.host,
                });
                Ok(guest)
            }
        }
    }
    pub(super) fn tap(&self) -> Result<Option<(&str, &str)>, RuntimeError> {
        match self {
            Self::User(_) => Ok(None),
            Self::Root { scope, profile } => {
                Ok(Some((scope.tap(), profile["guestMac"].as_str().ok_or_else(|| error("Missing root guest MAC"))?)))
            }
        }
    }
    pub(super) fn namespace(&self) -> Result<Option<vm_monitor::Namespace>, RuntimeError> {
        match self {
            Self::User(_) => Ok(None),
            Self::Root { scope, .. } => scope.monitor_namespace().map(Some).map_err(error),
        }
    }
    #[cfg(test)]
    pub(super) fn profile(&self) -> &Value {
        match self {
            Self::User(network) => network.profile(),
            Self::Root { profile, .. } => profile,
        }
    }
    pub(super) fn exited(&mut self) -> std::io::Result<Option<ExitStatus>> {
        match self {
            Self::User(network) => network.exited(),
            Self::Root { scope, .. } => scope.exited(),
        }
    }
    pub(super) async fn stop(&mut self) -> std::io::Result<()> {
        match self {
            Self::User(network) => network.stop().await,
            Self::Root { scope, .. } => scope.stop().await,
        }
    }
}
