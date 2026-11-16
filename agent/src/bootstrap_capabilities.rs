// SPDX-License-Identifier: FSL-1.1-ALv2
//! The node queries the actual agent executable before adding optional bootstrap arguments.
//! These describe parser support, never host isolation or native-tool readiness.
use serde::{Deserialize, Serialize};

pub const PROBE: &str = "bootstrap-probe";
pub const BROWSER_ARGUMENT: &str = "browser-sandbox";
pub const VSOCK_ARGUMENT: &str = "vm-vsock";
pub const VM_FILES_ARGUMENT: &str = "vm-files-only";
pub const VM_FILES_VERSION: u16 = 1;
pub const NATIVE_SYSCALL_VERSION: u16 = 1;

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    #[serde(default)]
    pub browser_policy: bool,
    #[serde(default)]
    pub exec_completion_version: u16,
    #[serde(default)]
    pub internal_ingress_version: u16,
    #[serde(default)]
    pub process_tools_version: u16,
    #[serde(default)]
    pub process_tools_host_cache_version: u16,
    #[serde(default)]
    pub process_browser_version: u16,
    #[serde(default)]
    pub process_desktop_version: u16,
    #[serde(default)]
    pub process_ssh_version: u16,
    #[serde(default)]
    pub vm_vsock: bool,
    #[serde(default)]
    pub vm_static_network: bool,
    #[serde(default)]
    pub vm_non_public_ipv4: Vec<String>,
    #[serde(default)]
    pub vm_init_version: u16,
    #[serde(default)]
    pub vm_boot_port: u32,
    #[serde(default)]
    pub vm_boot_marker: String,
    #[serde(default)]
    pub vm_boot_marker_path: String,
    #[serde(default)]
    pub vm_init_mode: String,
    #[serde(default)]
    pub vm_network_program: String,
    #[serde(default)]
    pub vm_packet_program: String,
    #[serde(default)]
    pub vm_monitor_version: u16,
    #[serde(default)]
    pub vm_monitor_namespace: bool,
    #[serde(default)]
    pub vm_monitor_mode: String,
    #[serde(default)]
    pub vm_service_port_base: u32,
    #[serde(default)]
    pub vm_stream_port_base: u32,
    #[serde(default)]
    pub vm_stream_version: u16,
    #[serde(default)]
    pub vm_files_version: u16,
    #[serde(default)]
    pub vm_shutdown_signals: bool,
    #[serde(default)]
    pub vm_firewall_version: u16,
    #[serde(default)]
    pub vm_firewall_directory: String,
    #[serde(default)]
    pub vm_firewall_program: String,
    #[serde(default)]
    pub vm_firewall_loader: String,
    #[serde(default)]
    pub vm_guest_policy_version: u16,
    #[serde(default)]
    pub vm_packet_proxy: bool,
    #[serde(default)]
    pub vm_packet_tap: bool,
    #[serde(default)]
    pub native_syscall_version: u16,
    #[serde(default)]
    pub vm_network_policy_version: u16,
    #[serde(default)]
    pub vm_packet_port: u32,
    /// The memory-sleep requests and resume this init answers (`vm_sleep.rs`); 0 for none.
    #[serde(default)]
    pub vm_sleep_version: u16,
}

#[cfg(feature = "server")]
pub fn probe() -> Capabilities {
    Capabilities {
        browser_policy: true,
        exec_completion_version: crate::exec_completion::VERSION,
        internal_ingress_version: crate::exec_ingress::VERSION,
        process_tools_version: crate::process_tool_policy::VERSION,
        process_tools_host_cache_version: crate::process_tool_policy::HOST_CACHE_VERSION,
        process_browser_version: crate::process_tool_policy::BROWSER_VERSION,
        process_desktop_version: crate::process_tool_policy::DESKTOP_VERSION,
        process_ssh_version: crate::process_tool_policy::SSH_VERSION,
        vm_vsock: true,
        vm_static_network: true,
        vm_non_public_ipv4: crate::transfer_target::non_public_ipv4_networks()
            .map(|(address, bits)| format!("{address}/{bits}"))
            .collect(),
        vm_init_version: crate::vm_boot::VERSION,
        vm_boot_port: crate::vm_boot::PORT,
        vm_boot_marker: String::from_utf8(crate::vm_boot::marker()).expect("fixed VM marker is UTF-8"),
        vm_boot_marker_path: crate::vm_boot::MARKER.into(),
        vm_init_mode: crate::vm_boot::INIT_MODE.into(),
        vm_network_program: crate::vm_boot::NETWORK_PROGRAM.into(),
        vm_packet_program: crate::vm_boot::PACKET_PROGRAM.into(),
        vm_monitor_version: crate::vm_monitor::VERSION,
        vm_monitor_namespace: true,
        vm_monitor_mode: crate::vm_monitor::MODE.into(),
        vm_service_port_base: crate::vm_boot::SERVICE_PORT_BASE,
        vm_stream_port_base: crate::vm_boot::STREAM_PORT_BASE,
        vm_stream_version: crate::vm_boot::STREAM_VERSION,
        vm_files_version: VM_FILES_VERSION,
        vm_shutdown_signals: true,
        vm_firewall_version: crate::vm_boot::FIREWALL_VERSION,
        vm_firewall_directory: crate::vm_boot::FIREWALL_DIRECTORY.into(),
        vm_firewall_program: crate::vm_boot::FIREWALL_PROGRAM.into(),
        vm_firewall_loader: crate::vm_boot::FIREWALL_LOADER.into(),
        vm_guest_policy_version: crate::vm_policy::VERSION,
        vm_packet_proxy: true,
        vm_packet_tap: true,
        native_syscall_version: NATIVE_SYSCALL_VERSION,
        vm_network_policy_version: crate::vm_boot::NETWORK_POLICY_VERSION,
        vm_packet_port: crate::vm_boot::PACKET_PORT,
        vm_sleep_version: crate::vm_sleep::VERSION,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earlier_agents_do_not_announce_host_owned_cache_support() {
        let earlier: Capabilities = serde_json::from_value(serde_json::json!({"processToolsVersion": 1})).unwrap();
        assert_eq!(earlier.process_tools_version, 1);
        assert_eq!(earlier.process_tools_host_cache_version, 0);
    }

    #[cfg(feature = "server")]
    #[test]
    fn the_current_agent_announces_exact_read_only_host_cache_support() {
        assert_eq!(probe().process_tools_host_cache_version, crate::process_tool_policy::HOST_CACHE_VERSION);
    }
}
