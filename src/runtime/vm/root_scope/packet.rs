// SPDX-License-Identifier: FSL-1.1-ALv2
//! Configure and admit the fixed ordinary packet process without privileged path writes.
//! The root scope owns launch, kernel identity and retirement behind the monitor lease.
use super::*;
use std::io::Write;
use tokio::io::AsyncWriteExt;

impl Scope {
    pub async fn start_packet_stack(&mut self, capacity: u16, deadline: Instant) -> Result<(), RuntimeError> {
        tokio::time::timeout_at(deadline, self.packet_ready(capacity))
            .await
            .map_err(|_| RuntimeError::Timeout("Root packet readiness exceeded its deadline".into()))?
    }

    async fn packet_ready(&mut self, capacity: u16) -> Result<(), RuntimeError> {
        let paths = self.packet_paths.take().ok_or_else(|| error("Packet launch is absent or already submitted"))?;
        let parent = self.child.as_ref().and_then(Child::id).ok_or_else(|| error("Root packet authority exited"))?;
        if capacity == 0 || paths.parent != parent {
            return Err(error("Packet launch requires its actual parent and positive capacity"));
        }
        // A root public guest's DNS is the stack's own gateway service and dials no host port;
        // only public IPv4 destinations outside the agent's boundary leave the stack.
        let configuration = serde_json::to_vec(&serde_json::json!({
            "parent": parent, "concurrency": capacity, "tapInterface":self.packet_tap,
            "egress": super::super::network::egress(true, &[], &[])
        }))
        .map_err(error)?;
        let limit = self.packet_profile["maxConfigurationBytes"]
            .as_u64()
            .ok_or_else(|| error("Packet configuration bound is absent"))?;
        if configuration.len() as u64 > limit {
            return Err(error("Packet configuration exceeds its executable bound"));
        }
        let configuration_argument = super::super::network::argument(&self.packet_profile, "configurationArgument")?;
        let descriptor_argument = super::super::network::argument(&self.packet_profile, "tapDescriptorArgument")?;
        if configuration_argument == descriptor_argument {
            return Err(error("Packet arguments collide"));
        }
        let resources = self.resources.as_mut().ok_or_else(|| error("Packet resources retired"))?;
        let descriptor =
            resources.configuration.take().ok_or_else(|| error("Packet configuration was already consumed"))?;
        let mut file = fs::File::from(descriptor);
        let info = file.metadata().map_err(error)?;
        if !info.is_file() || info.nlink() != 1 || info.mode() & 0o077 != 0 || info.len() != 0 {
            return Err(error("Packet configuration descriptor is not a new private regular file"));
        }
        file.write_all(&configuration).map_err(error)?;
        file.sync_all().map_err(error)?;
        drop(file);
        let plan = pryx_agent::vm_monitor::Plan {
            program: paths.program,
            arguments: vec![configuration_argument, paths.configuration.display().to_string()],
            directory: paths.directory,
            lock: paths.lease,
            parent,
            read: vec![paths.configuration],
            write: Vec::new(),
            role: pryx_agent::vm_monitor::Role::PacketTap { descriptor_argument },
        };
        // The launcher path is metadata about the fixed root-owned program, never a requested executable.
        if paths.launcher.parent() != plan.program.parent() {
            return Err(error("Packet programs have different protected staging roots"));
        }
        let mut bytes = serde_json::to_vec(&plan).map_err(error)?;
        bytes.push(b'\n');
        let metadata = self.packet_metadata.as_ref().ok_or_else(|| error("Missing packet parser metadata"))?;
        if bytes.len() > metadata.max_metadata_bytes {
            return Err(error("Packet launch exceeds its scope bound"));
        }
        let child = self.child.as_mut().ok_or_else(|| error("Packet authority retired"))?;
        let input = child.stdin.as_mut().ok_or_else(|| error("Missing packet lifetime channel"))?;
        input.write_all(&bytes).await.map_err(error)?;
        input.flush().await.map_err(error)?;
        let channel = resources.channel.as_ref().ok_or_else(|| error("Packet identity channel is absent"))?;
        let identity = channel
            .async_io(Interest::READABLE, || handoff::receive_identity(channel, metadata))
            .await
            .map_err(error)?;
        // Linux pidfd documentation names this anonymous-inode link; it exports no library constant.
        const PIDFD_LINK: &str = "anon_inode:[pidfd]";
        if fs::read_link(format!("/proc/self/fd/{}", identity.as_raw_fd())).map_err(error)? != Path::new(PIDFD_LINK) {
            return Err(error("Packet identity is not a kernel pidfd"));
        }
        resources.identity = Some(identity);
        if !self.packet_alive().map_err(error)? || self.exited().map_err(error)?.is_some() {
            return Err(error("Packet process exited during admission"));
        }
        Ok(())
    }

    pub fn packet_alive(&self) -> io::Result<bool> {
        let identity =
            self.resources.as_ref().and_then(|resources| resources.identity.as_ref()).ok_or_else(handoff::invalid)?;
        let mut descriptor = [rustix::event::PollFd::new(identity, rustix::event::PollFlags::IN)];
        Ok(rustix::event::poll(&mut descriptor, Some(&rustix::event::Timespec { tv_sec: 0, tv_nsec: 0 }))? == 0)
    }
}
