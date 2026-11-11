// SPDX-License-Identifier: FSL-1.1-ALv2
//! Native tool permissions are an explicit node administrator policy. A sandbox request cannot
//! supply a seccomp profile, acquire host capabilities, or request additional devices.
use super::RuntimeError;
use bollard::models::{ContainerCreateBody, DeviceMapping};
use std::{
    io::Read,
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
    path::Path,
};
#[derive(Default)]
pub(super) struct Policy {
    profile: Option<String>,
    fuse: bool,
    apparmor: Option<String>,
}
impl Policy {
    pub(super) fn load(path: Option<&Path>, fuse: bool, apparmor: Option<&str>) -> Result<Self, RuntimeError> {
        let enabled = std::fs::read_to_string("/sys/module/apparmor/parameters/enabled").is_ok_and(|v| v.trim() == "Y");
        if fuse && enabled && apparmor.is_none() {
            return Err(RuntimeError::Other(
                "Native FUSE requires an explicit enforcing AppArmor profile on this host".into(),
            ));
        }
        if let Some(name) = apparmor {
            if name.is_empty()
                || name.len() > 64
                || name == "unconfined"
                || !name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            {
                return Err(RuntimeError::Other("Native tools AppArmor profile name is invalid".into()));
            }
            let profiles = std::fs::read_to_string("/sys/kernel/security/apparmor/profiles")
                .map_err(|_| RuntimeError::Other("Cannot verify the host AppArmor profile".into()))?;
            if !profiles.lines().any(|line| line == format!("{name} (enforce)")) {
                return Err(RuntimeError::Other(
                    "Native tools AppArmor profile must already be loaded in enforcing mode".into(),
                ));
            }
        }
        let profile = path
            .map(|path| -> Result<String, RuntimeError> {
                if !path.is_absolute() {
                    return Err(RuntimeError::Other("Native tools seccomp profile must be absolute".into()));
                }
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW)
                    .open(path)
                    .map_err(|_| RuntimeError::Other("Cannot read native tools seccomp profile".into()))?;
                let meta = file
                    .metadata()
                    .map_err(|_| RuntimeError::Other("Cannot inspect native tools seccomp profile".into()))?;
                if !meta.is_file()
                    || meta.mode() & 0o022 != 0
                    || !(meta.uid() == 0 || meta.uid() == rustix::process::geteuid().as_raw())
                {
                    return Err(RuntimeError::Other(
                        "Native tools seccomp profile must be a protected host-owned file".into(),
                    ));
                }
                let mut text = String::new();
                file.take(1024 * 1024 + 1)
                    .read_to_string(&mut text)
                    .map_err(|_| RuntimeError::Other("Cannot read native tools seccomp profile".into()))?;
                if text.len() > 1024 * 1024
                    || !serde_json::from_str::<serde_json::Value>(&text)
                        .is_ok_and(|v| v.is_object() && v.get("defaultAction").is_some())
                {
                    return Err(RuntimeError::Other(
                        "Native tools seccomp profile must be a bounded JSON policy".into(),
                    ));
                }
                Ok(text)
            })
            .transpose()?;
        if fuse {
            if profile.is_none() {
                return Err(RuntimeError::Other("Native FUSE requires an explicit host seccomp profile".into()));
            }
            let device = std::fs::symlink_metadata("/dev/fuse")
                .map_err(|_| RuntimeError::Other("Native FUSE device is unavailable".into()))?;
            // Linux devices.txt: FUSE is character device 10:229; no arbitrary device path is accepted.
            if !device.file_type().is_char_device() || device.rdev() != rustix::fs::makedev(10, 229) {
                return Err(RuntimeError::Other("Native FUSE device is unavailable".into()));
            }
        }
        Ok(Self { profile, fuse, apparmor: apparmor.map(str::to_owned) })
    }
    pub(super) fn custom(&self) -> bool {
        self.fuse || self.profile.is_some() || self.apparmor.is_some()
    }
    pub(super) fn fuse(&self) -> bool {
        self.fuse
    }
    pub(super) fn apply(&self, body: &mut ContainerCreateBody) {
        let env = body.env.get_or_insert_default();
        env.retain(|v| !v.starts_with(&format!("{}=", pryx_agent::contract::FUSE_ENV)));
        env.push(format!("{}={}", pryx_agent::contract::FUSE_ENV, if self.fuse { "1" } else { "0" }));
        let host = body.host_config.as_mut().expect("trusted host config");
        if let Some(profile) = &self.profile {
            host.security_opt.get_or_insert_default().push(format!("seccomp={profile}"))
        }
        if let Some(profile) = &self.apparmor {
            host.security_opt.get_or_insert_default().push(format!("apparmor={profile}"))
        }
        if self.fuse {
            // Mountpoint directories need writable metadata despite a read-only image root.
            host.tmpfs.get_or_insert_default().insert("/mnt".into(), "rw,exec,nosuid,nodev,size=16m,mode=1777".into());
            host.devices = Some(vec![DeviceMapping {
                path_on_host: Some("/dev/fuse".into()),
                path_in_container: Some("/dev/fuse".into()),
                cgroup_permissions: Some("rw".into()),
            }])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn default_policy_overrides_an_image_fuse_flag_without_granting_permissions() {
        let mut body = ContainerCreateBody {
            env: Some(vec![format!("{}=1", pryx_agent::contract::FUSE_ENV)]),
            host_config: Some(Default::default()),
            ..Default::default()
        };
        Policy::default().apply(&mut body);
        assert_eq!(body.env.unwrap(), vec![format!("{}=0", pryx_agent::contract::FUSE_ENV)]);
        let host = body.host_config.unwrap();
        assert!(host.devices.is_none());
        assert!(host.security_opt.is_none());
        assert!(host.cap_add.is_none());
        assert!(Policy::load(None, true, None).is_err());
    }
    #[test]
    fn host_profile_requires_a_protected_regular_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("seccomp.json");
        std::fs::write(&path, r#"{"defaultAction":"SCMP_ACT_ERRNO","syscalls":[]}"#).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(Policy::load(Some(&path), false, None).is_ok());
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(Policy::load(Some(&link), false, None).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(Policy::load(Some(&path), false, None).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&path, "unconfined").unwrap();
        assert!(Policy::load(Some(&path), false, None).is_err());
    }
    #[test]
    fn fuse_policy_preserves_hardening_and_adds_only_its_device_and_metadata_mount() {
        let mut body = ContainerCreateBody {
            host_config: Some(bollard::models::HostConfig {
                readonly_rootfs: Some(true),
                security_opt: Some(vec!["no-new-privileges:true".into()]),
                ..Default::default()
            }),
            ..Default::default()
        };
        Policy { profile: Some("{}".into()), fuse: true, apparmor: Some("pryx-native-tools".into()) }.apply(&mut body);
        let host = body.host_config.unwrap();
        assert_eq!(host.readonly_rootfs, Some(true));
        assert!(host.cap_add.is_none());
        assert_eq!(host.privileged, None);
        assert!(host.security_opt.unwrap().contains(&"no-new-privileges:true".into()));
        let devices = host.devices.unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].path_on_host.as_deref(), Some("/dev/fuse"));
        assert_eq!(devices[0].cgroup_permissions.as_deref(), Some("rw"));
        assert!(host.tmpfs.unwrap()["/mnt"].contains("size=16m"));
    }
    #[test]
    fn caller_cannot_select_an_unconfined_or_unverified_apparmor_profile() {
        for name in ["unconfined", "", "../profile", "profile*", "profile\n"] {
            assert!(Policy::load(None, false, Some(name)).is_err())
        }
        assert!(Policy::load(None, false, Some("pryx-missing-profile")).is_err());
    }
}
