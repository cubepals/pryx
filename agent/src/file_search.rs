// SPDX-License-Identifier: FSL-1.1-ALv2
//! Searches and tar transfers walk persistent data through directory descriptors. Symlinks,
//! special entries, traversal and oversized scans fail explicitly, before outside data opens.
use crate::{
    context::{Agent, Failure},
    paths::DataDir,
};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use futures_util::TryStreamExt;
use serde::Deserialize;
use std::{
    io::{self, Read},
    sync::Arc,
};
use tokio_util::io::{ReaderStream, StreamReader};

use crate::contract::{ARCHIVE_BYTES as MAX_ARCHIVE, SEARCH_BYTES as MAX_SCAN, SEARCH_ENTRIES as MAX_ENTRIES};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub(crate) struct Search {
    pattern: String,
    #[serde(default)]
    paths: Vec<String>,
    text: Option<String>,
}
#[derive(Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub(crate) struct Path {
    path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub(crate) struct Mode {
    path: String,
    mode: u32,
}
pub(crate) fn routes() -> Router<Arc<Agent>> {
    Router::new()
        .route("/_pryx/files/find", post(find))
        .route("/_pryx/files/grep", post(grep))
        .route("/_pryx/files/replace", post(replace))
        .route("/_pryx/files/chmod", post(chmod))
        .route("/_pryx/files/archive", get(archive).put(extract).layer(DefaultBodyLimit::disable()))
}
fn invalid(message: &str) -> Failure {
    Failure(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request", message.into())
}
fn walk(data: &DataDir, path: &str, found: &mut Vec<String>) -> io::Result<()> {
    visit(data, path, found, false, &mut 0)
}
fn visit(data: &DataDir, path: &str, found: &mut Vec<String>, directories: bool, count: &mut usize) -> io::Result<()> {
    for entry in data.list(path)? {
        *count += 1;
        if *count > MAX_ENTRIES {
            return Err(io::Error::other("File count exceeds scan budget"));
        }
        let name = if path == "." { entry.name } else { format!("{path}/{}", entry.name) };
        match entry.kind {
            "file" => found.push(name),
            "directory" => {
                if directories {
                    found.push(name.clone())
                }
                visit(data, &name, found, directories, count)?;
            }
            _ => return Err(io::Error::other("Search and archives refuse links and special files")),
        }
    }
    Ok(())
}
static ARCHIVES: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(crate::contract::ARCHIVE_CONCURRENCY);
fn archive_slot() -> Result<tokio::sync::SemaphorePermit<'static>, Failure> {
    ARCHIVES
        .try_acquire()
        .map_err(|_| Failure(StatusCode::TOO_MANY_REQUESTS, "archive_capacity", "Archive capacity reached".into()))
}
fn spool(data: &DataDir) -> io::Result<std::fs::File> {
    use std::os::fd::AsRawFd;
    let dir = data.directory_file(".")?;
    tempfile::tempfile_in(format!("/proc/self/fd/{}", dir.as_raw_fd()))
}
fn glob(pattern: &str) -> Result<globset::GlobMatcher, Failure> {
    Ok(globset::GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .map_err(|_| invalid("Invalid glob"))?
        .compile_matcher())
}
async fn find(State(agent): State<Arc<Agent>>, Json(body): Json<Search>) -> Result<Json<serde_json::Value>, Failure> {
    let matcher = glob(&body.pattern)?;
    agent
        .file_tasks
        .clone()
        .spawn_blocking(move || {
            let mut files = Vec::new();
            walk(&agent.data, ".", &mut files)?;
            let files: Vec<_> = files.into_iter().filter(|p| matcher.is_match(p)).collect();
            let answer = serde_json::json!({"paths":files});
            if answer.to_string().len() > crate::contract::MAX_REQUEST_BYTES {
                return Err(invalid("Find exceeds response budget"));
            }
            Ok(Json(answer))
        })
        .await
        .map_err(|_| invalid("Search interrupted"))?
}
fn scan(data: &DataDir, mut body: Search, replace: bool) -> Result<serde_json::Value, Failure> {
    let pattern = regex::RegexBuilder::new(&body.pattern)
        .size_limit(crate::contract::CAPTURE_BYTES)
        .build()
        .map_err(|_| invalid("Invalid or excessive regular expression"))?;
    let mut paths = Vec::new();
    if body.paths.is_empty() {
        body.paths.push(".".into());
    }
    for path in &body.paths {
        if data.stat(path)?.kind == "directory" {
            walk(data, path, &mut paths)?;
        } else {
            paths.push(path.clone());
        }
        if paths.len() > MAX_ENTRIES {
            return Err(invalid("File count exceeds scan budget"));
        }
    }
    if paths.len() > MAX_ENTRIES {
        return Err(invalid("File count exceeds scan budget"));
    }
    paths.sort();
    paths.dedup();
    let mut scanned = 0;
    let mut matches = Vec::new();
    let mut response_bytes = 0;
    let mut count = 0;
    for path in paths {
        let input = data.read(&path)?;
        let metadata = input.metadata()?;
        let size = metadata.len();
        if scanned + size > MAX_SCAN || size > crate::contract::CAPTURE_BYTES as u64 {
            return Err(invalid("Search exceeds byte budget"));
        }
        let mut text = String::new();
        input.take(crate::contract::CAPTURE_BYTES as u64 + 1).read_to_string(&mut text)?;
        scanned += text.len() as u64;
        if text.len() > crate::contract::CAPTURE_BYTES || scanned > MAX_SCAN {
            return Err(invalid("Search exceeds byte budget"));
        }
        if replace {
            let replacement = body.text.as_deref().ok_or_else(|| invalid("Replacement text required"))?;
            let n = pattern.find_iter(&text).count();
            if n > 0 {
                let mut value = String::new();
                let mut at = 0;
                for matched in pattern.find_iter(&text) {
                    let prefix = &text[at..matched.start()];
                    if value.len().saturating_add(prefix.len()).saturating_add(replacement.len())
                        > crate::contract::CAPTURE_BYTES
                    {
                        return Err(invalid("Replacement exceeds file budget"));
                    }
                    value.push_str(prefix);
                    value.push_str(replacement);
                    at = matched.end();
                }
                let tail = &text[at..];
                if value.len() + tail.len() > crate::contract::CAPTURE_BYTES {
                    return Err(invalid("Replacement exceeds file budget"));
                }
                value.push_str(tail);
                data.write_atomic(&path, value.as_bytes(), true)?;
                use std::os::unix::fs::PermissionsExt;
                data.chmod(&path, metadata.permissions().mode() & 0o777)?;
                count += n;
            }
        } else {
            for (line, text) in text.lines().enumerate() {
                if pattern.is_match(text) {
                    let matched = serde_json::json!({"path":path,"line":line+1,"text":text});
                    response_bytes += serde_json::to_vec(&matched).map_err(io::Error::other)?.len() + 1;
                    if response_bytes > crate::contract::MAX_REQUEST_BYTES {
                        return Err(invalid("Grep exceeds response budget"));
                    }
                    matches.push(matched);
                }
            }
            if serde_json::to_vec(&matches).map_err(io::Error::other)?.len() > crate::contract::MAX_REQUEST_BYTES {
                return Err(invalid("Grep exceeds response budget"));
            }
        }
    }
    Ok(if replace { serde_json::json!({"replacements":count}) } else { serde_json::json!({"matches":matches}) })
}
async fn grep(State(agent): State<Arc<Agent>>, Json(body): Json<Search>) -> Result<Json<serde_json::Value>, Failure> {
    agent
        .file_tasks
        .clone()
        .spawn_blocking(move || scan(&agent.data, body, false).map(Json))
        .await
        .map_err(|_| invalid("Search interrupted"))?
}
async fn replace(
    State(agent): State<Arc<Agent>>,
    Json(body): Json<Search>,
) -> Result<Json<serde_json::Value>, Failure> {
    agent
        .file_tasks
        .clone()
        .spawn_blocking(move || scan(&agent.data, body, true).map(Json))
        .await
        .map_err(|_| invalid("Replace interrupted"))?
}
async fn chmod(State(agent): State<Arc<Agent>>, Json(body): Json<Mode>) -> Result<StatusCode, Failure> {
    if body.mode > 0o777 {
        return Err(invalid("Mode must contain only permission bits"));
    }
    agent.data.chmod(&body.path, body.mode)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn archive(State(agent): State<Arc<Agent>>, Query(path): Query<Path>) -> Result<Body, Failure> {
    let permit = archive_slot()?;
    let (file, permit) = agent
        .file_tasks
        .clone()
        .spawn_blocking(move || -> io::Result<(std::fs::File, tokio::sync::SemaphorePermit<'static>)> {
            let mut entries = Vec::new();
            visit(&agent.data, &path.path, &mut entries, true, &mut 0)?;
            let mut archive = tar::Builder::new(spool(&agent.data)?);
            for entry in entries {
                let info = agent.data.stat(&entry)?;
                let mut file = if info.kind == "directory" {
                    agent.data.directory_file(&entry)?
                } else {
                    agent.data.read(&entry)?
                };
                let metadata = file.metadata()?;
                if info.kind == "file"
                    && metadata.len()
                        > MAX_ARCHIVE.saturating_sub(archive.get_ref().metadata()?.len()).saturating_sub(1024)
                {
                    return Err(io::Error::other("Archive exceeds byte budget"));
                }
                let relative = if path.path == "." {
                    entry.as_str()
                } else {
                    entry.strip_prefix(&format!("{}/", path.path)).ok_or(io::ErrorKind::InvalidInput)?
                };
                let mut header = tar::Header::new_gnu();
                use std::os::unix::fs::PermissionsExt;
                header.set_mode(metadata.permissions().mode() & 0o777);
                if info.kind == "directory" {
                    header.set_entry_type(tar::EntryType::Directory);
                    header.set_size(0);
                    header.set_cksum();
                    archive.append_data(&mut header, relative, io::empty())?;
                } else {
                    header.set_size(metadata.len());
                    header.set_cksum();
                    archive.append_data(&mut header, relative, &mut file)?;
                }
                if archive.get_ref().metadata()?.len() > MAX_ARCHIVE {
                    return Err(io::Error::other("Archive exceeds byte budget"));
                }
            }
            let mut file = archive.into_inner()?;
            if file.metadata()?.len() > MAX_ARCHIVE {
                return Err(io::Error::other("Archive exceeds byte budget"));
            }
            use std::io::Seek;
            file.rewind()?;
            Ok((file, permit))
        })
        .await
        .map_err(|_| invalid("Archive interrupted"))??;
    use futures_util::StreamExt;
    let stream = ReaderStream::new(tokio::fs::File::from_std(file)).map(move |chunk| {
        let _keep = &permit;
        chunk
    });
    Ok(Body::from_stream(stream))
}
async fn extract(State(agent): State<Arc<Agent>>, Query(path): Query<Path>, body: Body) -> Result<StatusCode, Failure> {
    let permit = archive_slot()?;
    let mut file = tokio::fs::File::from_std(spool(&agent.data)?);
    let mut input = StreamReader::new(body.into_data_stream().map_err(io::Error::other));
    let copied = tokio::io::copy(&mut tokio::io::AsyncReadExt::take(&mut input, MAX_ARCHIVE + 1), &mut file).await?;
    if copied > MAX_ARCHIVE {
        return Err(invalid("Archive exceeds byte budget"));
    }
    let mut file = file.into_std().await;
    agent
        .file_tasks
        .clone()
        .spawn_blocking(move || -> io::Result<()> {
            let _permit = permit;
            use std::io::{Seek, Write};
            file.rewind()?;
            let mut archive = tar::Archive::new(file);
            let mut size: u64 = 0;
            let mut modes = Vec::new();
            agent.data.mkdir(&path.path)?;
            for (count, entry) in archive.entries()?.enumerate() {
                if count >= MAX_ENTRIES {
                    return Err(io::Error::other("Archive exceeds entry budget"));
                }
                let mut entry = entry?;
                let name = entry.path()?;
                if name
                    .components()
                    .any(|p| !matches!(p, std::path::Component::Normal(_) | std::path::Component::CurDir))
                {
                    return Err(io::ErrorKind::PermissionDenied.into());
                }
                let name = name.to_str().ok_or(io::ErrorKind::InvalidInput)?;
                let target = format!("{}/{name}", path.path);
                if entry.header().entry_type().is_dir() {
                    agent.data.mkdir(&target)?;
                    modes.push((target, entry.header().mode()? & 0o777));
                } else if entry.header().entry_type().is_file() {
                    size = size.checked_add(entry.size()).ok_or(io::ErrorKind::InvalidData)?;
                    if size > MAX_ARCHIVE {
                        return Err(io::Error::other("Expanded archive exceeds byte budget"));
                    }
                    let mut output = agent.data.write(&target)?;
                    io::copy(&mut entry, &mut output)?;
                    output.flush()?;
                    agent.data.chmod(&target, entry.header().mode()? & 0o777)?;
                } else {
                    return Err(io::ErrorKind::PermissionDenied.into());
                }
            }
            for (path, mode) in modes.into_iter().rev() {
                agent.data.chmod(&path, mode)?;
            }
            Ok(())
        })
        .await
        .map_err(|_| invalid("Extract interrupted"))??;
    Ok(StatusCode::NO_CONTENT)
}
