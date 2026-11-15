// SPDX-License-Identifier: FSL-1.1-ALv2
//! Select only the host-owned VM network path. Ordinary users retain their confined packet
//! helper; root hostname policy uses its leased TAP namespace and existing gateways. Public
//! root traffic crosses the separately filtered bridge into a confined ordinary packet stack.
use super::*;
use crate::protocol::EgressPolicy;
use pryx_agent::{vm_boot, vm_monitor};
use serde_json::Value;
use std::process::ExitStatus;
use tokio::time::Instant;

pub(super) enum Network {
    User(Box<super::super::network::Network>),
    Root { scope: Box<super::super::root_scope::Scope>, profile: Value, unrestricted: bool },
}

impl Network {
    pub(super) async fn root(
        settings: &Settings,
        bundle: &Bundle,
        boots: &Path,
        lease: &Path,
        policy: Option<EgressPolicy>,
        deadline: Instant,
    ) -> Result<Self, RuntimeError> {
        if !bundle.bootstrap().metadata.vm_static_network || !bundle.bootstrap().metadata.vm_monitor_namespace {
            return Err(error("Root TAP requires advertised static guest and leased monitor namespace support"));
        }
        let mut profile =
            super::super::packet_profile::load(&bundle.root().join("bin/network-proxy"), deadline).await?;
        let program = bundle.root().join("bin/root-scope");
        let plugins = bundle.root().join("bin/root-cni");
        let paths = super::super::root_scope::Paths {
            program: &program,
            plugins: &plugins,
            boots,
            lease,
            owner: settings.monitor_owner.ok_or_else(|| error("Missing fixed root monitor identities"))?,
        };
        let unrestricted = policy.is_none();
        let scope = if let Some(policy) = policy {
            super::super::root_scope::Scope::start(paths, policy, settings.ingress_connections, deadline).await?
        } else {
            if !bundle.bootstrap().metadata.vm_packet_tap {
                return Err(error("Root public TAP requires advertised confined packet queue support"));
            }
            let agent = bundle.root().join("bin/vm-launcher");
            let packet = bundle.root().join("bin/network-proxy");
            let mut scope = super::super::root_scope::Scope::start_packet(
                paths,
                super::super::root_scope::BridgePrograms { agent: &agent, packet: &packet },
                deadline,
            )
            .await?;
            scope.start_packet_stack(settings.ingress_connections, deadline).await?;
            scope
        };
        let root_profile = scope.profile().as_object().ok_or_else(|| error("Missing root TAP profile"))?;
        let services = profile.as_object_mut().ok_or_else(|| error("Missing packet service metadata"))?;
        services.extend(root_profile.iter().map(|(key, value)| (key.clone(), value.clone())));
        Ok(Self::Root { scope: Box::new(scope), profile, unrestricted })
    }
    pub(super) fn guest(&self) -> Result<vm_boot::Network, RuntimeError> {
        match self {
            Self::User(network) => network.guest(),
            Self::Root { scope, profile, unrestricted } => {
                let (http, tls, dns) = if *unrestricted {
                    let port = |name: &str| {
                        profile[name]
                            .as_u64()
                            .and_then(|port| u16::try_from(port).ok())
                            .filter(|port| *port != 0)
                            .ok_or_else(|| error("Root packet profile omitted its service port"))
                    };
                    (port("http")?, port("tls")?, port("dns")?)
                } else {
                    scope.ports()
                };
                let ports = crate::runtime::egress::listeners::Ports { http, tls, dns };
                let mut guest = super::super::packet_profile::guest(profile, &ports, *unrestricted)?;
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
            Self::Root { scope, profile, .. } => {
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
