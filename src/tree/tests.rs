// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Real-filesystem no-follow traversal, copy and disk-floor regressions. No runtime or
//! filesystem response is simulated here; only callback interleavings are controlled.

use std::io::Read;
use std::os::unix::fs::PermissionsExt;

use super::walk::{ST_BLOCK_BYTES, walk_at_most};
use super::*;
use rustix::fs::{FileType, Mode};
use std::path::Path;
use std::{
    fs::{self, File},
    io,
};

fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    fs::create_dir_all(root.join("world/region")).unwrap();
    fs::write(root.join("server.properties"), "level-name=world\n").unwrap();
    fs::write(root.join("world/level.dat"), b"level").unwrap();
    fs::write(root.join("world/region/r.0.0.mca"), vec![7u8; 70_000]).unwrap();
    fs::create_dir_all(root.join("libraries/x")).unwrap();
    fs::write(root.join("libraries/x/lib.jar"), b"jar").unwrap();
    dir
}

fn me() -> (u32, u32) {
    (rustix::process::getuid().as_raw(), rustix::process::getgid().as_raw())
}

#[test]
fn a_walk_visits_parents_first_in_order_and_leaves_out_what_is_excluded() {
    let dir = tree();
    let mut seen = Vec::new();
    let walked = walk(&dir.path().join("data"), &["libraries".into()], &mut |e| {
        seen.push(e.path.to_string_lossy().into_owned());
        Ok(())
    })
    .unwrap();
    assert_eq!(seen, ["", "server.properties", "world", "world/level.dat", "world/region", "world/region/r.0.0.mca"]);
    assert_eq!((walked.dirs, walked.files, walked.links), (3, 3, 0));
    assert_eq!(walked.bytes, 17 + 5 + 70_000);
}

#[test]
fn a_directory_of_more_names_than_a_walk_takes_is_refused_before_any_is_visited() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    fs::create_dir_all(root.join("world")).unwrap();
    for n in 0..8 {
        fs::write(root.join(format!("world/r.{n}.mca")), b"").unwrap();
    }
    // The root, world/ and eight files: exactly the limit, which a walk still takes.
    let walked = walk_at_most(&root, &[], 10, &mut |_| Ok(())).unwrap();
    assert_eq!(walked.entries(), 10);
    fs::write(root.join("world/r.8.mca"), b"").unwrap();
    let mut visited = Vec::new();
    let refused = walk_at_most(&root, &[], 10, &mut |e| {
        visited.push(e.path.to_string_lossy().into_owned());
        Ok(())
    })
    .unwrap_err();
    assert_eq!(refused.to_string(), "more than 10 entries");
    assert_eq!(visited, ["", "world"], "refused as world/'s names were read, before any of them");
}

#[test]
fn exclusion_is_by_top_level_name_only() {
    let dir = tree();
    let root = dir.path().join("data");
    fs::create_dir_all(root.join("world/libraries")).unwrap();
    let mut seen = Vec::new();
    walk(&root, &["libraries".into()], &mut |e| {
        seen.push(e.path.to_string_lossy().into_owned());
        Ok(())
    })
    .unwrap();
    assert!(seen.contains(&"world/libraries".to_owned()), "a nested name of the same spelling stays");
    assert!(!seen.iter().any(|p| p.starts_with("libraries")));
}

#[test]
fn links_are_reported_never_followed_and_fifos_are_never_opened() {
    let dir = tree();
    let root = dir.path().join("data");
    let outside = dir.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret"), b"host secret").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("world/escape")).unwrap();
    std::os::unix::fs::symlink(outside.join("secret"), root.join("secret-link")).unwrap();
    rustix::fs::mknodat(rustix::fs::CWD, root.join("pipe").as_path(), FileType::Fifo, Mode::from_raw_mode(0o600), 0)
        .unwrap();
    let mut read = Vec::new();
    let walked = walk(&root, &[], &mut |e| {
        if let Kind::File(f) = e.kind {
            let mut s = Vec::new();
            f.read_to_end(&mut s).unwrap();
            read.push(s);
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(walked.links, 2);
    assert_eq!(walked.skipped, 1, "the FIFO");
    assert!(!read.iter().any(|r| r == b"host secret"), "nothing outside the root was read");
}

#[test]
fn a_directory_swapped_for_a_link_mid_walk_is_never_followed() {
    // What a plugin can do while the sandbox runs: swap a directory it owns for a link to the
    // host, over and over, hoping a walk looks before the swap and opens after it.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let outside = dir.path().join("outside");
    fs::create_dir_all(root.join("world")).unwrap();
    fs::write(root.join("world/level.dat"), b"level").unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("level.dat"), b"host secret").unwrap();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let swapper = {
        let (root, outside, stop) = (root.clone(), outside.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = fs::rename(root.join("world"), root.join("world.real"));
                let _ = std::os::unix::fs::symlink(&outside, root.join("world"));
                let _ = fs::remove_file(root.join("world"));
                let _ = fs::rename(root.join("world.real"), root.join("world"));
            }
        })
    };
    for _ in 0..300 {
        let _ = walk(&root, &[], &mut |e| {
            if let Kind::File(f) = e.kind {
                let mut body = Vec::new();
                f.read_to_end(&mut body)?;
                assert_ne!(body, b"host secret", "read through a swapped-in link");
            }
            Ok(())
        });
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    swapper.join().unwrap();
}

/// The disk usage as it was measured before, by path: what an honest tree still comes to.
fn usage_by_path(root: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    let mut total = fs::symlink_metadata(root).unwrap().blocks() * ST_BLOCK_BYTES;
    let mut stack = vec![root.to_owned()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap().map(Result::unwrap) {
            let meta = entry.metadata().unwrap();
            total += meta.blocks() * ST_BLOCK_BYTES;
            if meta.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    total
}

#[test]
fn disk_usage_counts_an_honest_tree_as_it_always_did() {
    let dir = tree();
    let root = dir.path().join("data");
    fs::write(root.join("world/empty"), b"").unwrap();
    fs::hard_link(root.join("world/level.dat"), root.join("level.dat.bak")).unwrap();
    std::os::unix::fs::symlink("world/level.dat", root.join("link")).unwrap();
    let fifo = root.join("pipe");
    rustix::fs::mknodat(rustix::fs::CWD, fifo.as_path(), FileType::Fifo, Mode::from_raw_mode(0o600), 0).unwrap();
    let used = disk_usage(&root).unwrap();
    assert!(used > 0);
    assert_eq!(used, usage_by_path(&root));
}

#[test]
fn disk_usage_never_counts_or_enters_a_directory_swapped_for_a_link() {
    // The swap above, while the data is measured. A measure that opens directories by path
    // resolves `world` again for every directory beneath it, and follows the link whenever
    // the swap has it in place: what is behind it would count as the workload's.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let outside = dir.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("host.img"), vec![1u8; 8 << 20]).unwrap();
    for n in 0..64 {
        fs::create_dir_all(root.join(format!("world/DIM{n}"))).unwrap();
        // The same names behind the link, each holding the one 8 MiB file.
        fs::create_dir_all(outside.join(format!("DIM{n}"))).unwrap();
        fs::hard_link(outside.join("host.img"), outside.join(format!("DIM{n}/host.img"))).unwrap();
    }
    let honest = disk_usage(&root).unwrap();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let swapper = {
        let (root, outside, stop) = (root.clone(), outside.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = fs::rename(root.join("world"), root.join("world.real"));
                let _ = std::os::unix::fs::symlink(&outside, root.join("world"));
                let _ = fs::remove_file(root.join("world"));
                let _ = fs::rename(root.join("world.real"), root.join("world"));
            }
        })
    };
    let most = (0..300).filter_map(|_| disk_usage(&root).ok()).max().unwrap_or(0);
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    swapper.join().unwrap();
    // Honest, give or take what the swap renames: never the 8 MiB behind the link, which
    // counts only once the measure enters it.
    assert!(most < honest + (1 << 20), "measured {most} bytes of a {honest}-byte tree");
}

#[test]
fn a_copy_holds_the_same_bytes_and_tamed_modes() {
    let dir = tree();
    let root = dir.path().join("data");
    fs::set_permissions(root.join("server.properties"), fs::Permissions::from_mode(0o4777)).unwrap();
    std::os::unix::fs::symlink("world/level.dat", root.join("link")).unwrap();
    let dst = dir.path().join("copy");
    let copied = copy_tree(&root, &dst, Some(me()), &["libraries".into()], &mut |_| Ok(())).unwrap();
    assert_eq!(fs::read(dst.join("world/region/r.0.0.mca")).unwrap(), vec![7u8; 70_000]);
    assert_eq!(fs::read_link(dst.join("link")).unwrap(), Path::new("world/level.dat"));
    assert!(!dst.join("libraries").exists());
    let mode = fs::metadata(dst.join("server.properties")).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode, 0o755);
    let mtime = |p: &Path| fs::metadata(p).unwrap().modified().unwrap();
    assert_eq!(mtime(&dst.join("world/level.dat")), mtime(&root.join("world/level.dat")));
    assert_eq!(copied.walked.files, 3);
    // Whichever the filesystem allows; both give the same bytes.
    assert!(matches!(copied.method, Method::Reflink | Method::Copy));
}

#[test]
fn a_copy_stops_when_the_disk_has_no_room() {
    let dir = tree();
    let mut told = Vec::new();
    let refused = copy_tree(&dir.path().join("data"), &dir.path().join("copy"), Some(me()), &[], &mut |copied| {
        told.push(copied);
        if copied > 20 { Err("the disk is under its floor".into()) } else { Ok(()) }
    })
    .unwrap_err();
    assert_eq!(NoRoom::of(&refused), Some("the disk is under its floor"), "{refused}");
    // Files in byte order: lib.jar, server.properties, then level.dat, which is one too many.
    assert_eq!(told, [3, 3 + 17, 3 + 17 + 5]);
    assert!(!dir.path().join("copy/world/region").exists(), "nothing after the refusal");
    assert_eq!(NoRoom::of(&io::Error::other("a disk's own failure")), None);
}

#[test]
fn a_tree_stating_more_than_a_restore_takes_is_refused_before_it_is_copied() {
    let dir = tree();
    let root = dir.path().join("data");
    // Sparse: the size is stated, with no blocks behind it for the disk's usage to count.
    File::create(root.join("world/huge")).unwrap().set_len(MAX_BYTES + 1).unwrap();
    let dst = dir.path().join("copy");
    let refused = copy_tree(&root, &dst, Some(me()), &[], &mut |_| Ok(())).unwrap_err();
    assert!(NoRoom::of(&refused).is_some_and(|m| m.contains("more than a restore unpacks")), "{refused}");
    assert!(disk_usage(&dst).unwrap() < 4 << 20, "none of it was copied");
}

#[test]
fn a_long_file_is_stopped_at_the_floor_while_it_is_copied() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    fs::create_dir_all(&root).unwrap();
    File::create(root.join("huge")).unwrap().set_len(1 << 30).unwrap();
    let mut told = Vec::new();
    let refused = copy_tree(&root, &dir.path().join("copy"), Some(me()), &[], &mut |copied| {
        told.push(copied);
        if copied > 256 << 20 { Err("the disk is under its floor".into()) } else { Ok(()) }
    })
    .unwrap_err();
    assert_eq!(NoRoom::of(&refused), Some("the disk is under its floor"), "{refused}");
    if reflink_supported(dir.path()) {
        // Shared, the file takes no space, and is told whole once cloned.
        assert_eq!(told, [1 << 30]);
    } else {
        // Copied, its apparent bytes still count, and `room` is asked in bounded steps.
        assert_eq!(told.last(), Some(&((256 << 20) + ROOM_STEP)), "stopped partway: {told:?}");
        assert!(fs::metadata(dir.path().join("copy/huge")).unwrap().len() <= (256 << 20) + ROOM_STEP);
    }
}
