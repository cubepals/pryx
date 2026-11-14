// SPDX-License-Identifier: FSL-1.1-ALv2
//! Private VM specifications survive daemon restarts outside workload data and exports.
//! Journal contents may contain credentials, are never logged, and require one owned 0600
//! regular file. An interrupted launch never becomes a signed clean guest completion here.
use super::*;
use std::io::Read;

const MAX_BYTES: u64 = 16 * 1024 * 1024;

fn identifier(id: &str) -> bool {
    id.strip_prefix("vm-")
        .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok_and(|value| value.simple().to_string() == id))
}
fn read(path: &Path) -> Result<Entry, RuntimeError> {
    let file =
        OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path).map_err(error)?;
    let info = file.metadata().map_err(error)?;
    if !info.is_file()
        || info.uid() != rustix::process::geteuid().as_raw()
        || info.mode() & 0o077 != 0
        || info.nlink() != 1
        || info.len() > MAX_BYTES
    {
        return Err(error("Unsafe private VM journal"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes).map_err(error)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(error("VM private journal exceeds its bound"));
    }
    // A deserializer's error can quote an offending credential-bearing value.
    let entry: Entry = serde_json::from_slice(&bytes).map_err(|_| error("Invalid private VM journal"))?;
    if !identifier(&entry.id) || path.file_stem() != Some(std::ffi::OsStr::new(&entry.id)) {
        return Err(error("VM journal filename differs from its identity"));
    }
    launch::resources(&entry.spec)?;
    agent::key(&entry.spec)?;
    Ok(entry)
}
pub(super) fn load(root: &Path) -> Result<BTreeMap<String, Entry>, RuntimeError> {
    let mut entries = BTreeMap::new();
    for path in fs::read_dir(root.join("entries")).map_err(error)? {
        let path = path.map_err(error)?.path();
        if path.extension().is_none_or(|value| value != "json") {
            continue;
        }
        let mut entry = read(&path)?;
        if entries.values().any(|previous: &Entry| previous.spec.name == entry.spec.name) {
            return Err(error("Duplicate VM sandbox name in private journals"));
        }
        if entry.started.is_some() && entry.finished.is_none() {
            entry.code = UNKNOWN_EXIT_CODE;
            entry.finished = Some(crate::clock::now());
        }
        if let Some(snapshot) = &mut entry.agent_state {
            snapshot.activity.streams = 0;
        }
        entries.insert(entry.id.clone(), entry);
    }
    Ok(entries)
}
impl VmRuntime {
    pub(super) fn persist_entry(&self, entry: &Entry) -> Result<(), RuntimeError> {
        if !identifier(&entry.id) {
            return Err(error("Invalid VM journal identity"));
        }
        let bytes = serde_json::to_vec(entry).map_err(|_| error("Cannot encode private VM journal"))?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(error("VM private journal exceeds its bound"));
        }
        crate::durable::write_atomic(&self.state.root.join("entries").join(format!("{}.json", entry.id)), &bytes, 0o600)
            .map_err(error)
    }
    pub(super) fn save_entry(&self, entry: Entry) -> Result<(), RuntimeError> {
        let mut entries = self.state.entries.lock().unwrap();
        let result = self.persist_entry(&entry);
        // Actual exit remains observed even when persistence fails. On restart an older
        // unfinished launch is uncertain and must never be read as a clean completion.
        entries.insert(entry.id.clone(), entry);
        result
    }
    pub(super) fn forget_entry(&self, id: &str) -> Result<(), RuntimeError> {
        if !identifier(id) {
            return Err(error("Invalid VM journal identity"));
        }
        let directory = self.state.root.join("entries");
        match fs::remove_file(directory.join(format!("{id}.json"))) {
            Ok(()) => (),
            Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => (),
            Err(problem) => return Err(error(problem)),
        }
        File::open(&directory).and_then(|directory| directory.sync_all()).map_err(error)?;
        self.state.entries.lock().unwrap().remove(id);
        Ok(())
    }
}

pub(super) mod timestamp {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};
    pub(in crate::runtime::vm::instance) fn serialize<S: Serializer>(
        value: &Option<OffsetDateTime>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.map(|value| value.format(&Rfc3339)).transpose().map_err(serde::ser::Error::custom)?.serialize(serializer)
    }
    pub(in crate::runtime::vm::instance) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<OffsetDateTime>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .map(|value| OffsetDateTime::parse(&value, &Rfc3339))
            .transpose()
            .map_err(serde::de::Error::custom)
    }
}
