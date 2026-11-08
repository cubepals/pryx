// SPDX-License-Identifier: FSL-1.1-ALv2
//! S3 mounts run unmodified Mountpoint S3 or rclone. FUSE exists only under host-approved
//! nested namespaces; this adapter never substitutes a directory mirror or writes credentials.
use super::{
    command,
    state::{self, MountEntry},
    types::{MountSpec, MountView},
};
use crate::{
    context::{Agent, Failure},
    process::Process,
};
use std::os::fd::AsRawFd;
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};

fn executable(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|path| path.join(name).is_file()))
}
fn normalize(agent: &Agent, path: &str) -> Result<String, Failure> {
    let path = Path::new(path);
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return Err(state::failure("invalid_request", "Mount path must be absolute without dot components"));
    }
    let normalized: PathBuf = path.components().collect();
    let (_root, relative): (&Path, &Path) = if let Ok(relative) = normalized.strip_prefix(&agent.processes.data) {
        (&agent.processes.data, relative)
    } else {
        (
            Path::new("/mnt"),
            normalized
                .strip_prefix("/mnt")
                .map_err(|_| state::failure("invalid_request", "Mount paths must be below /mnt or persistent data"))?,
        )
    };
    let relative = relative
        .to_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| state::failure("invalid_request", "Mount cannot cover a root directory"))?;
    if relative.split('/').any(|s| s == crate::contract::STATE_DIRECTORY) {
        return Err(state::failure("invalid_request", "Mount cannot cover agent state"));
    }
    Ok(normalized.to_string_lossy().into_owned())
}
fn target(agent: &Agent, path: &str) -> Result<std::fs::File, Failure> {
    let normalized = Path::new(path);
    let (root, relative): (&Path, &Path) = if let Ok(relative) = normalized.strip_prefix(&agent.processes.data) {
        (&agent.processes.data, relative)
    } else {
        (Path::new("/mnt"), normalized.strip_prefix("/mnt").expect("validated mount root"))
    };
    let relative = relative.to_str().expect("validated UTF-8 path");
    let owner = crate::paths::DataDir::open(root)?;
    let original = owner.directory_handle(relative, true)?;
    let original_stat = original.metadata()?;
    let pinned = format!("/proc/self/fd/{}", original.as_raw_fd());
    rustix::mount::mount_bind(&pinned, &pinned).map_err(std::io::Error::from)?;
    let file = owner.directory_handle(relative, false)?;
    use std::os::unix::fs::MetadataExt;
    let actual = file.metadata()?;
    if original_stat.dev() != actual.dev() || original_stat.ino() != actual.ino() {
        return Err(state::failure("mount_target_changed", "Mount target changed during preparation"));
    }
    rustix::mount::mount_change(
        format!("/proc/self/fd/{}", file.as_raw_fd()),
        rustix::mount::MountPropagationFlags::SHARED,
    )
    .map_err(std::io::Error::from)?;
    // The existing no-follow walk pins the target through exec, even if another caller renames it.
    rustix::io::fcntl_setfd(&file, rustix::io::FdFlags::empty()).map_err(std::io::Error::from)?;
    Ok(file)
}
fn source(spec: &MountSpec) -> Result<(String, String), Failure> {
    let url = reqwest::Url::parse(&spec.bucket)
        .map_err(|_| state::failure("invalid_request", "Bucket must be s3://bucket/prefix"))?;
    let bucket = url.host_str().unwrap_or_default();
    if url.scheme() != "s3"
        || bucket.is_empty()
        || !bucket.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(state::failure("invalid_request", "Bucket must be s3://bucket/prefix"));
    }
    let prefix = percent_encoding::percent_decode_str(url.path().trim_start_matches('/'))
        .decode_utf8()
        .map_err(|_| state::failure("invalid_request", "S3 prefix is not UTF-8"))?
        .into_owned();
    if prefix.contains('\0') {
        return Err(state::failure("invalid_request", "S3 prefix contains NUL"));
    }
    Ok((bucket.into(), if prefix.is_empty() || prefix.ends_with('/') { prefix } else { format!("{prefix}/") }))
}
fn credentials(spec: &MountSpec) -> Result<BTreeMap<String, String>, Failure> {
    let c = &spec.credentials;
    if c.access_key_id.is_empty()
        || c.secret_access_key.is_empty()
        || [
            Some(&c.access_key_id),
            Some(&c.secret_access_key),
            c.session_token.as_ref(),
            c.region.as_ref(),
            c.endpoint.as_ref(),
        ]
        .into_iter()
        .flatten()
        .any(|s| s.contains('\0'))
    {
        return Err(state::failure("invalid_request", "Invalid mount credentials"));
    }
    if let Some(endpoint) = &c.endpoint {
        let url = reqwest::Url::parse(endpoint)
            .map_err(|_| state::failure("invalid_request", "S3 endpoint must use HTTP or HTTPS"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(state::failure("invalid_request", "S3 endpoint must use HTTP or HTTPS without credentials"));
        }
    }
    Ok(BTreeMap::from([
        ("AWS_ACCESS_KEY_ID".into(), c.access_key_id.clone()),
        ("AWS_SECRET_ACCESS_KEY".into(), c.secret_access_key.clone()),
        ("AWS_SESSION_TOKEN".into(), c.session_token.clone().unwrap_or_default()),
        ("AWS_EC2_METADATA_DISABLED".into(), "true".into()),
        ("AWS_CONFIG_FILE".into(), "/dev/null".into()),
        ("AWS_SHARED_CREDENTIALS_FILE".into(), "/dev/null".into()),
    ]))
}
pub(crate) async fn list(agent: &Agent) -> Result<Vec<MountView>, Failure> {
    state::native(agent)?;
    let state = state::get(agent)?;
    let entries = state.mounts.lock().await;
    Ok(entries
        .iter()
        .map(|entry| {
            let mut view = entry.view.clone();
            if !entry.process.view().running {
                view.state = "dead".into()
            }
            view
        })
        .collect())
}
pub(crate) async fn initialize(agent: &Agent, specs: Vec<MountSpec>) -> Result<Vec<MountView>, Failure> {
    state::native(agent)?;
    if !crate::fuse_namespace::enabled() {
        return Err(state::failure(
            "unsupported",
            "FUSE mounts are unsupported on this container host; native FUSE policy is disabled",
        ));
    }
    if specs.is_empty() || specs.len() > 16 {
        return Err(state::failure("invalid_request", "One to sixteen mounts are required"));
    }
    let state = state::get(agent)?;
    let mut entries = state.mounts.lock().await;
    for mut spec in specs {
        spec.path = normalize(agent, &spec.path)?;
        let fingerprint =
            ring::digest::digest(&ring::digest::SHA256, &serde_json::to_vec(&spec).expect("mount serializes"))
                .as_ref()
                .to_vec();
        let path = spec.path.clone();
        if let Some(existing) = entries.iter().find(|e| e.view.path == path) {
            if existing.fingerprint != fingerprint {
                return Err(state::failure("mount_conflict", "Mount path has another source or policy"));
            }
            if !existing.process.view().running {
                return Err(state::failure("mount_died", "Mount worker exited; restart the sandbox explicitly"));
            }
            continue;
        }
        let (bucket, prefix) = source(&spec)?;
        let mut env = credentials(&spec)?;
        let directory = target(agent, &path)?;
        env.insert("HOME".into(), state.directory.path().to_string_lossy().into_owned());
        let target = format!("/proc/self/fd/{}", directory.as_raw_fd());
        let (backend, mut argv) = if executable("mount-s3") {
            let mut argv = vec![
                "mount-s3".into(),
                bucket.clone(),
                target,
                "--foreground".into(),
                "--log-directory".into(),
                state.directory.path().join("mount-logs").to_string_lossy().into_owned(),
            ];
            if !prefix.is_empty() {
                argv.extend(["--prefix".into(), prefix.clone()]);
            }
            if let Some(endpoint) = &spec.credentials.endpoint {
                argv.extend(["--endpoint-url".into(), endpoint.clone(), "--force-path-style".into()]);
            }
            if let Some(region) = &spec.credentials.region {
                argv.extend(["--region".into(), region.clone()]);
            }
            ("mountpoint-s3", argv)
        } else if executable("rclone") {
            env.extend(BTreeMap::from([
                ("RCLONE_CONFIG_PRYX_TYPE".into(), "s3".into()),
                ("RCLONE_CONFIG_PRYX_PROVIDER".into(), "Other".into()),
                ("RCLONE_CONFIG_PRYX_ACCESS_KEY_ID".into(), spec.credentials.access_key_id.clone()),
                ("RCLONE_CONFIG_PRYX_SECRET_ACCESS_KEY".into(), spec.credentials.secret_access_key.clone()),
                ("RCLONE_CONFIG_PRYX_SESSION_TOKEN".into(), spec.credentials.session_token.clone().unwrap_or_default()),
            ]));
            if let Some(endpoint) = &spec.credentials.endpoint {
                env.insert("RCLONE_CONFIG_PRYX_ENDPOINT".into(), endpoint.clone());
            }
            if let Some(region) = &spec.credentials.region {
                env.insert("RCLONE_CONFIG_PRYX_REGION".into(), region.clone());
            }
            (
                "rclone",
                vec![
                    "rclone".into(),
                    "mount".into(),
                    format!("pryx:{bucket}/{prefix}"),
                    target,
                    "--config".into(),
                    "/dev/null".into(),
                    // The approved shared bind mount already occupies this verified target.
                    "--allow-non-empty".into(),
                    "--vfs-cache-mode".into(),
                    "writes".into(),
                    "--vfs-write-back".into(),
                    "0s".into(),
                    "--vfs-cache-max-size".into(),
                    "64M".into(),
                    "--cache-dir".into(),
                    state.directory.path().join("rclone-cache").to_string_lossy().into_owned(),
                    "--log-level".into(),
                    "ERROR".into(),
                ],
            )
        } else {
            return Err(state::failure(
                "tool_unavailable",
                "Install Mountpoint S3 or rclone in the native tools image",
            ));
        };
        if spec.read_only {
            argv.push("--read-only".into());
        } else if backend == "mountpoint-s3" {
            argv.extend(["--allow-overwrite".into(), "--allow-delete".into()]);
        }
        let process = command::start(agent, argv, env, None, true)?;
        let mut starting = command::Starting(Some(process.clone()));
        mounted(&process, Path::new(&path)).await?;
        entries.push(MountEntry {
            view: MountView {
                bucket: spec.bucket,
                path,
                read_only: spec.read_only,
                backend: backend.into(),
                state: "mounted".into(),
            },
            process,
            fingerprint,
            _directory: directory,
        });
        starting.0 = None;
    }
    state.mounts_ready.store(true, std::sync::atomic::Ordering::Release);
    Ok(entries.iter().map(|e| e.view.clone()).collect())
}
async fn mounted(process: &Arc<Process>, path: &Path) -> Result<(), Failure> {
    let work = async {
        loop {
            let mounts = procfs::process::Process::myself()
                .and_then(|p| p.mountinfo())
                .map_err(|_| state::failure("mount_failed", "Cannot verify FUSE mount"))?;
            if mounts.iter().any(|m| m.mount_point == path && m.fs_type.starts_with("fuse")) {
                return Ok(());
            }
            if !process.view().running {
                return Err(state::failure(
                    "mount_failed",
                    "FUSE worker refused this mount; check endpoint and host policy",
                ));
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(30), work)
        .await
        .map_err(|_| state::failure("mount_timeout", "FUSE mount did not become ready"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec() -> MountSpec {
        serde_json::from_value(serde_json::json!({"bucket":"s3://bucket/prefix%20one","path":"/mnt/data","readOnly":true,"credentials":{"accessKeyId":"key","secretAccessKey":"secret","endpoint":"http://storage.invalid:9000","region":"test"}})).unwrap()
    }
    #[test]
    fn s3_sources_and_credentials_have_no_implicit_url_authority() {
        let mut s = spec();
        assert_eq!(source(&s).unwrap_or_else(|_| panic!("valid S3 source")), ("bucket".into(), "prefix one/".into()));
        assert_eq!(
            credentials(&s).unwrap_or_else(|_| panic!("valid S3 credentials"))["AWS_EC2_METADATA_DISABLED"],
            "true"
        );
        for url in [
            "https://bucket/path",
            "s3://user:secret@bucket/path",
            "s3://bucket/path?signature=secret",
            "s3://bucket/path#fragment",
            "s3://bucket/%00",
        ] {
            s.bucket = url.into();
            assert!(source(&s).is_err(), "{url}")
        }
        s = spec();
        for endpoint in
            ["file:///etc/passwd", "http://key:secret@storage.invalid", "http://storage.invalid/?signature=secret"]
        {
            s.credentials.endpoint = Some(endpoint.into());
            assert!(credentials(&s).is_err())
        }
        let view = MountView {
            bucket: "s3://bucket/prefix".into(),
            path: "/mnt/data".into(),
            read_only: true,
            backend: "rclone".into(),
            state: "mounted".into(),
        };
        assert!(!serde_json::to_string(&view).unwrap().contains("secret"));
    }
}
