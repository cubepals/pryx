// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Real root jailer boots using the same verified bundle, disk staging, generated configuration,
//! bootstrap and agent transport as ordinary launches. This is not production root placement,
//! a TAP/firewall test, or proof that the daemon's service has the required host authority.
use super::*;
use crate::runtime::vm::{disk, image, jail};
use hyper::Method;
use pryx_agent::{process, vm_monitor, vm_shutdown};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;

struct Launch {
    jail: jail::Jail,
    disk: PathBuf,
    identity: u64,
    _directory: tempfile::TempDir,
    plan: vm_monitor::Plan,
    boot: pryx_agent::vm_boot::Boot,
    channel: PathBuf,
    serial: PathBuf,
    deadline: tokio::time::Instant,
}

struct Host<'a> {
    root: &'a Path,
    bundle: &'a Bundle,
    owner: (u32, u32),
}
impl Host<'_> {
    async fn prepare(&self, spec: &ContainerSpec, image: &Prepared) -> Launch {
        let lifetime = self.root.join("leases/monitor.lock");
        let disk = disk::prepare(
            self.bundle.filesystems(),
            &spec.data_dir,
            &lifetime,
            spec.storage_bytes,
            launch::BOOT_TIMEOUT,
            0,
        )
        .await
        .unwrap();
        let identity = fs::metadata(&disk.path).unwrap().ino();
        let id = uuid::Uuid::new_v4().simple().to_string();
        let monitor = self.bundle.root().join("bin/firecracker");
        let base = self.root.join("jails");
        let mut readonly = BTreeMap::from([
            (PathBuf::from("kernel/vmlinux"), self.bundle.root().join("kernel/vmlinux")),
            (PathBuf::from("bootstrap/bootstrap.erofs"), self.bundle.root().join("bootstrap/bootstrap.erofs")),
        ]);
        for (index, layer) in image.layers.iter().enumerate() {
            readonly.insert(PathBuf::from(format!("layers/{index}")), layer.clone());
        }
        let mut jail = jail::Jail::prepare(jail::Request {
            base: &base,
            id: &id,
            monitor: &monitor,
            disk: &disk.path,
            lease: &lifetime,
            owner: self.owner,
            readonly: &readonly,
        })
        .unwrap();
        let root = jail.root().to_owned();
        let staged = Prepared {
            digest: image.digest.clone(),
            configuration: image.configuration.clone(),
            layers: image.layers.iter().enumerate().map(|(index, _)| root.join(format!("layers/{index}"))).collect(),
        };
        let directory = boot_directory(&self.root.join("boots")).unwrap();
        let channel = root.join("vsock");
        let serial = directory.path().join("serial");
        let config = directory.path().join("monitor.json");
        let deadline = tokio::time::Instant::now() + launch::BOOT_TIMEOUT;
        let (memory, cpus, uid, gid) = launch::resources(spec).unwrap();
        let configuration = configuration::configure(
            self.bundle,
            &staged,
            &root.join(disk::FILE),
            &channel,
            configuration::Destination { output: &config, jail: Some(&root) },
            (memory, cpus),
            deadline,
        )
        .await
        .unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o444)).unwrap();
        jail.add_readonly(Path::new("monitor.json"), &config).unwrap();
        let boot = guest::boot(spec, &id, &staged, &configuration, (uid, gid)).unwrap();
        let plan = vm_monitor::Plan {
            program: self.bundle.root().join("bin/jailer"),
            arguments: vec!["--api-sock".into(), "/api".into(), "--config-file".into(), "/monitor.json".into()],
            directory: directory.path().into(),
            lock: lifetime.clone(),
            parent: std::process::id(),
            read: Vec::new(),
            write: Vec::new(),
            role: vm_monitor::Role::Jailer { uid: self.owner.0, gid: self.owner.1, id, base, monitor },
        };
        Launch { jail, disk: disk.path, identity, _directory: directory, plan, boot, channel, serial, deadline }
    }
    async fn boot(&self, spec: &ContainerSpec, image: &Prepared, retained: bool) {
        let mut launch = self.prepare(spec, image).await;
        let mut monitor = Machine::launch(
            &self.bundle.root().join("bin/vm-launcher"),
            &launch.plan,
            &launch.channel,
            &launch.serial,
            &launch.boot,
            launch.deadline,
            1024 * 1024,
        )
        .await
        .unwrap();
        let endpoint = agent::endpoint(spec).unwrap();
        let mut forward = Forwarder::start(
            &self.bundle.root().join("bin/vm-transport"),
            &launch.channel,
            &[super::super::forward::Binding::service(
                endpoint.ip(),
                endpoint.port(),
                spec.agent.as_ref().unwrap().agent_port,
            )
            .unwrap()],
            8,
            launch.deadline,
        )
        .await
        .unwrap();
        let result = async {
            monitor.delivered(launch.deadline).await?;
            launch::ready(&mut monitor, spec, launch.deadline).await?;
            monitor.bootstrapped(launch.deadline).await?;
            probe(spec, &self.root.join("host-only"), retained).await
        }
        .await;
        if let Err(problem) = result {
            diagnostics::failed(&mut monitor, &launch.serial, spec).await;
            forward.stop().await.unwrap();
            launch.jail.retire().unwrap();
            panic!("Actual root jailed guest refused: {problem}");
        }
        launch.observe(&monitor, self.owner);
        self.shutdown(&launch, &mut monitor, spec).await;
        forward.stop().await.unwrap();
        launch.jail.retire().unwrap();
        assert_eq!(fs::metadata(&launch.disk).unwrap().uid(), rustix::process::Uid::ROOT.as_raw());
        assert_eq!(fs::metadata(&launch.disk).unwrap().ino(), launch.identity);
    }
    async fn shutdown(&self, launch: &Launch, monitor: &mut Machine, spec: &ContainerSpec) {
        let agent = spec.agent.as_ref().unwrap();
        let request = vm_shutdown::request(&agent::key(spec).unwrap(), &launch.boot.nonce, &agent.sandbox, agent.epoch);
        super::super::transport::request(
            &self.bundle.root().join("bin/vm-transport"),
            &json!({"socket":launch.channel, "port":vm_shutdown::PORT, "shutdown":request}),
            launch::BOOT_TIMEOUT,
            pryx_agent::vm_boot::MAX_BYTES,
        )
        .await
        .unwrap();
        tokio::time::timeout(launch::BOOT_TIMEOUT, monitor.wait()).await.unwrap().unwrap();
        monitor.finish_serial().await.unwrap();
        assert!(!monitor.serial_truncated());
        diagnostics::retain(&launch.serial, &spec.env.iter().map(|(_, value)| value.clone()).collect::<Vec<_>>());
        runtime::shutdown_proof(
            &launch.serial,
            &agent::key(spec).unwrap().public_key(),
            &launch.boot.nonce,
            &agent.sandbox,
            agent.epoch,
        )
        .unwrap();
    }
}

async fn probe(spec: &ContainerSpec, host_file: &Path, retained: bool) -> Result<(), RuntimeError> {
    if retained {
        let file =
            agent::raw(spec, Method::GET, "/_pryx/files/content?path=retained", Value::Null, Duration::from_secs(5))
                .await?;
        assert_eq!(file, b"root-jailer", "Cold root boot must retain the original disk contents");
    }
    let start = process::Start {
        command: process::Command::Argv(vec![
            "/bin/sh".into(),
            "-c".into(),
            "test ! -e \"$1\" || exit 1; if kill -0 \"$2\" 2>/dev/null; then exit 1; fi; printf root-jailer > /data/retained; uname -r > /data/kernel".into(),
            "probe".into(),
            host_file.display().to_string(),
            std::process::id().to_string(),
        ]),
        env: Default::default(),
        cwd: None,
        stdin: false,
        pty: false,
        cols: None,
        rows: None,
        timeout: Some(5),
    };
    let started: Value = serde_json::from_slice(
        &agent::raw(
            spec,
            Method::POST,
            "/_pryx/processes",
            serde_json::to_value(start).unwrap(),
            Duration::from_secs(5),
        )
        .await?,
    )
    .unwrap();
    let id = started["id"].as_str().unwrap();
    let waited: Value = serde_json::from_slice(
        &agent::raw(spec, Method::GET, &format!("/_pryx/processes/{id}/wait"), Value::Null, Duration::from_secs(6))
            .await?,
    )
    .unwrap();
    assert_eq!(waited["code"], 0);
    let file = agent::raw(spec, Method::GET, "/_pryx/files/content?path=retained", Value::Null, Duration::from_secs(5))
        .await?;
    assert_eq!(file, b"root-jailer");
    let kernel =
        agent::raw(spec, Method::GET, "/_pryx/files/content?path=kernel", Value::Null, Duration::from_secs(5)).await?;
    assert_ne!(
        String::from_utf8(kernel).unwrap().trim(),
        fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires actual root, KVM and the verified VM archive; optional VM CI runs this test"]
async fn actual_jailer_boots_both_images_and_recovers_the_original_disk() {
    assert!(rustix::process::geteuid().is_root());
    let _ = rustls::crypto::ring::default_provider().install_default();
    let root = tempfile::tempdir().unwrap();
    crate::durable::write_atomic(&root.path().join("host-only"), uuid::Uuid::new_v4().as_bytes(), 0o600).unwrap();
    for name in ["cache", "downloads", "images", "leases", "boots", "jails"] {
        directory(&root.path().join(name)).unwrap();
    }
    let (artifact, server) = tests::artifact_server().await;
    let bundle = Bundle::acquire(&root.path().join("cache"), &root.path().join("downloads"), &artifact).await.unwrap();
    let host = Host {
        root: root.path(),
        bundle: &bundle,
        owner: (
            std::env::var("PRYX_VM_ROOT_TEST_UID").unwrap().parse().unwrap(),
            std::env::var("PRYX_VM_ROOT_TEST_GID").unwrap().parse().unwrap(),
        ),
    };
    let inputs: Vec<Value> =
        serde_json::from_str(include_str!("../../../../templates/vm-tools/image-inputs.json")).unwrap();
    for (index, input) in inputs.iter().enumerate() {
        let data = root.path().join(format!("data{index}"));
        directory(&data).unwrap();
        let spec = tests::spec(&data, input["reference"].as_str().unwrap());
        let image = image::prepare(
            &bundle.root().join("bin/pryx-vm-image"),
            &image::Request {
                component: bundle.root().join("filesystem"),
                manifest_sha256: bundle.filesystems().identity().into(),
                cache: root.path().join("images"),
                image: spec.image.clone(),
                timeout_seconds: 120,
            },
        )
        .await
        .unwrap();
        for generation in 0..2 {
            eprintln!("Actual jailed VM image {index}, generation {generation}");
            host.boot(&spec, &image, generation != 0).await;
        }
    }
    server.abort();
}

impl Launch {
    fn observe(&mut self, monitor: &Machine, owner: (u32, u32)) {
        let pid = monitor.pid().unwrap();
        let status = procfs::process::Process::new(pid.try_into().unwrap()).unwrap().status().unwrap();
        assert_eq!(status.euid, owner.0);
        // The jailer pivots a private mount namespace; its root symlink can render as "/"
        // to this observer. The actual directory identity must still equal our staged jail.
        let observed = fs::metadata(format!("/proc/{pid}/root")).unwrap();
        let expected = fs::metadata(self.jail.root()).unwrap();
        assert_eq!((observed.dev(), observed.ino()), (expected.dev(), expected.ino()));
        assert_ne!(fs::read_link(format!("/proc/{pid}/ns/mnt")).unwrap(), fs::read_link("/proc/self/ns/mnt").unwrap());
        assert!(lock(&self.plan.lock).is_err(), "The actual jailed monitor retains its inherited lease");
        assert!(self.jail.retire().is_err(), "A live monitor cannot authorize disk ownership recovery");
    }
}
