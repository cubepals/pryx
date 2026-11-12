// SPDX-License-Identifier: FSL-1.1-ALv2
//! Live unchanged X11 owner/client and same-UID neighbour acceptance. Explicit CI supplies
//! the staged component; this design test enables no product desktop capability.
use pryx_agent::{process_policy, process_tool_policy};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs,
    io::{BufRead, BufReader, Read},
    os::{
        linux::net::SocketAddrExt,
        unix::net::{SocketAddr, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const INPUT: &str = "PRYX_X11_LIVE_WORKER";
#[derive(Clone, Deserialize, Serialize)]
struct Input {
    role: String,
    root: PathBuf,
    component: PathBuf,
    base: PathBuf,
    display: Option<String>,
    peers: Vec<u32>,
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn policy(input: &Input) -> process_policy::Policy {
    process_policy::Policy {
        data: input.root.join("data"),
        memory_bytes: 512 * 1024 * 1024,
        address_space_bytes: 64 * 1024 * 1024 * 1024,
        open_files: 256,
        processes: 4096,
        cpu_seconds: 60,
        nice: 1,
        network_off: false,
        require_ipc_scope: Some(true),
        tools: Some(process_tool_policy::Policy {
            cache: input.component.clone(),
            downloads: input.root.join("downloads"),
            scratch: input.root.join("scratch"),
            artifact: None,
            unavailable: None,
            browser_on: false,
        }),
    }
}
fn command(input: &Input, program: &str) -> Command {
    let mut command = Command::new(input.component.join("bin").join(program));
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", input.root.join("scratch"))
        .env("XAUTHORITY", input.root.join("scratch/Xauthority"));
    if let Some(display) = &input.display {
        command.env("DISPLAY", display);
    }
    command
}
fn worker(input: &Input) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "live_x11_worker", "--ignored", "--nocapture"])
        .env_clear()
        .env(INPUT, serde_json::to_string(input).unwrap())
        .current_dir(input.root.join("data"));
    command
}
fn server(input: &Input) {
    process_policy::desktop(&policy(input)).unwrap();
    let status = command(input, "Xvfb")
        .args(["-displayfd", "1", "-nolisten", "tcp", "-screen", "0", "800x600x24"])
        .arg("-auth")
        .arg(input.root.join("scratch/Xauthority"))
        .arg("-xkbdir")
        .arg(input.component.join("payload/usr/share/X11/xkb"))
        .status()
        .unwrap();
    std::process::exit(status.code().unwrap_or(1));
}
fn owner(input: &Input) {
    use ring::rand::SecureRandom;
    process_policy::scope().unwrap();
    let mut key = [0u8; 16];
    ring::rand::SystemRandom::new().fill(&mut key).unwrap();
    let cookie = key.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    let mut setup = command(input, "xauth");
    assert!(setup.args(["add", ":0", ".", &cookie]).output().unwrap().status.success());
    let mut nested = input.clone();
    nested.role = "server".into();
    let executable = std::env::current_exe().unwrap();
    let mut launch = Command::new(input.base.join("helpers/bwrap"));
    launch
        .env_clear()
        .env("LD_LIBRARY_PATH", input.base.join("libraries/lib"))
        .args([
            "--unshare-user",
            "--unshare-pid",
            "--unshare-ipc",
            "--unshare-uts",
            "--die-with-parent",
            "--new-session",
            "--cap-drop",
            "ALL",
        ])
        .args([
            "--uid",
            &rustix::process::geteuid().as_raw().to_string(),
            "--gid",
            &rustix::process::getegid().as_raw().to_string(),
        ]);
    for path in ["/usr", "/bin", "/lib", "/lib64", "/etc"] {
        if Path::new(path).exists() {
            launch.args(["--ro-bind", path, path]);
        }
    }
    launch
        .args(["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp", "--tmpfs", "/dev/shm", "--tmpfs", "/usr/bin"])
        .args(["--ro-bind", "/usr/bin/dash", "/usr/bin/sh"])
        .arg("--ro-bind")
        .arg(input.component.join("payload/usr/bin/xkbcomp"))
        .arg("/usr/bin/xkbcomp");
    for path in [&input.component, &executable] {
        launch.arg("--ro-bind").arg(path).arg(path);
    }
    launch
        .arg("--bind")
        .arg(&input.root)
        .arg(&input.root)
        .arg("--chdir")
        .arg(input.root.join("data"))
        .args(["--setenv", INPUT, &serde_json::to_string(&nested).unwrap()])
        .arg(&executable)
        .args(["--exact", "live_x11_worker", "--ignored", "--nocapture"])
        .stdout(Stdio::piped())
        .stderr(fs::File::create(input.root.join("data/server.stderr")).unwrap());
    let mut running = Running(launch.spawn().unwrap());
    let mut output = BufReader::new(running.0.stdout.take().unwrap());
    let display = loop {
        let mut line = String::new();
        assert!(output.read_line(&mut line).unwrap() > 0, "Display exited before readiness");
        if let Ok(number) = line.trim().parse::<u32>() {
            break format!(":{number}");
        }
    };
    let mut client = input.clone();
    client.display = Some(display.clone());
    assert!(command(&client, "xauth").args(["add", &display, ".", &cookie]).output().unwrap().status.success());
    process_policy::enter(&policy(input)).unwrap();
    let result = command(&client, "xdotool").arg("getmouselocation").output().unwrap();
    assert!(result.status.success(), "Own X11 client: {}", String::from_utf8_lossy(&result.stderr));
    let address = SocketAddr::from_abstract_name(format!("/tmp/.X11-unix/X{}", &display[1..])).unwrap();
    assert!(UnixStream::connect_addr(&address).is_ok());
    fs::write(input.root.join("data/ready.json"), serde_json::to_vec(&json!({"display":display,"uid":rustix::process::geteuid().as_raw(),"ownClient":String::from_utf8_lossy(&result.stdout),"ownAbstractConnect":true})).unwrap()).unwrap();
    while !input.root.join("data/stop").exists() {
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn neighbour(input: &Input) {
    process_policy::enter(&policy(input)).unwrap();
    assert!(fs::read(input.root.parent().unwrap().join("owner/data/ready.json")).is_err());
    let display = input.display.as_ref().unwrap();
    let number = &display[1..];
    let address = SocketAddr::from_abstract_name(format!("/tmp/.X11-unix/X{number}")).unwrap();
    let abstract_error = UnixStream::connect_addr(&address).unwrap_err();
    let mut aliases = Vec::new();
    for peer in &input.peers {
        let path = format!("/proc/{peer}/root/tmp/.X11-unix/X{number}");
        let error = UnixStream::connect(&path).unwrap_err();
        aliases.push(json!({"path":path,"error":error.to_string()}));
    }
    let report = json!({"uid":rustix::process::geteuid().as_raw(),"abstractDenied":abstract_error.to_string(),"procRootAliasesDenied":aliases});
    fs::write(input.root.join("data/neighbour.json"), serde_json::to_vec(&report).unwrap()).unwrap();
}
#[test]
#[ignore = "subprocess entry for the explicit live X11 design test"]
fn live_x11_worker() {
    let input: Input = serde_json::from_str(&std::env::var(INPUT).unwrap()).unwrap();
    match input.role.as_str() {
        "server" => server(&input),
        "owner" => owner(&input),
        "neighbour" => neighbour(&input),
        _ => panic!("Unknown fixture role"),
    }
}
fn descendants(pid: u32, result: &mut Vec<u32>) {
    result.push(pid);
    let text = fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")).unwrap();
    for child in text.split_whitespace() {
        descendants(child.parse().unwrap(), result);
    }
}
#[test]
#[ignore = "explicit staged X11 tools, user namespaces and enforced filesystem/private IPC"]
fn private_temporary_x11_allows_own_client_and_denies_same_uid_neighbour() {
    assert!(pryx_agent::process_capabilities::probe().filesystem);
    assert!(pryx_agent::process_capabilities::private_ipc_probe().supported());
    assert!(!rustix::process::geteuid().is_root());
    let root = tempfile::tempdir().unwrap();
    for name in ["owner", "neighbour"] {
        for directory in ["data", "scratch", "downloads"] {
            fs::create_dir_all(root.path().join(name).join(directory)).unwrap();
        }
    }
    let mut input = Input {
        role: "owner".into(),
        root: root.path().join("owner"),
        component: PathBuf::from(std::env::var_os("PRYX_DESKTOP_TEST_COMPONENT").unwrap()).canonicalize().unwrap(),
        base: PathBuf::from(std::env::var_os("PRYX_DESKTOP_TEST_BASE").unwrap()).canonicalize().unwrap(),
        display: None,
        peers: Vec::new(),
    };
    let mut owner = Running(
        worker(&input)
            .stdout(Stdio::null())
            .stderr(fs::File::create(root.path().join("owner.stderr")).unwrap())
            .spawn()
            .unwrap(),
    );
    let ready = input.root.join("data/ready.json");
    let deadline = Instant::now() + Duration::from_secs(15);
    while !ready.exists() {
        if owner.0.try_wait().unwrap().is_some() || Instant::now() > deadline {
            panic!(
                "Live display failed: {}; server: {}",
                fs::read_to_string(root.path().join("owner.stderr")).unwrap(),
                fs::read_to_string(input.root.join("data/server.stderr")).unwrap_or_default()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let owner_report: serde_json::Value = serde_json::from_slice(&fs::read(&ready).unwrap()).unwrap();
    descendants(owner.0.id(), &mut input.peers);
    assert!(input.peers.len() > 2, "Actual namespaced server descendants must exist");
    let number = &owner_report["display"].as_str().unwrap()[1..];
    let live_aliases = input
        .peers
        .iter()
        .filter(|pid| UnixStream::connect(format!("/proc/{pid}/root/tmp/.X11-unix/X{number}")).is_ok())
        .count();
    assert!(live_aliases > 0, "The unconfined controller must reach an actual proc-root socket alias");
    input.display = Some(owner_report["display"].as_str().unwrap().into());
    input.root = root.path().join("neighbour");
    input.role = "neighbour".into();
    let mut neighbour = Running(worker(&input).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = neighbour.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "Neighbour probe deadline");
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut diagnostic = String::new();
    neighbour.0.stderr.take().unwrap().read_to_string(&mut diagnostic).unwrap();
    assert!(status.success(), "Neighbour: {diagnostic}");
    let neighbour_report: serde_json::Value =
        serde_json::from_slice(&fs::read(input.root.join("data/neighbour.json")).unwrap()).unwrap();
    assert_eq!(owner_report["uid"], neighbour_report["uid"]);
    let report = json!({"owner":owner_report,"neighbour":neighbour_report,"filesystemEnforced":true,"scope":"Live Xvfb/xdotool design only; no headful browser or installed desktop acceptance"});
    fs::write(root.path().join("owner/data/stop"), b"stop").unwrap();
    if let Some(path) = std::env::var_os("PRYX_DESKTOP_LIVE_REPORT") {
        fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    println!("{report}");
}
