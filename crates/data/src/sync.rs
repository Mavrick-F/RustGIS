//! The files a layer was read from, as they were when it was read ([`FileSync`]): what a layer
//! kept in the project compares with its source before its edits are written back to it, so
//! that writing them never overwrites a change made to the file since (#1385).

use serde::{Deserialize, Serialize};

/// The files of a source as they were at a moment: each one's name, size, modification time
/// and SHA-256 hash, and the files that weren't there. A Shapefile is its `.shp`, `.shx`,
/// `.dbf`, `.prj` and `.cpg` (the ones its reader opens); a GeoJSON file is itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSync {
    /// Each file by its name beside the source (`parcels.dbf`), with its stamp, or `None` for
    /// one that wasn't there, so a `.prj` added since counts as a change too.
    pub files: Vec<(String, Option<FileStamp>)>,
}

/// One file as it was.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileStamp {
    pub len: u64,
    /// When it was last written, in nanoseconds since 1970 (`None` where the system doesn't
    /// say). Only a shortcut: the hash decides whether the file changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<u64>,
    /// The SHA-256 of its bytes, in hexadecimal.
    pub sha256: String,
}

/// Whether `path` names a source whose files a [`FileSync`] records: a Shapefile or a
/// `.geojson` file. A `.json` file isn't one yet: it may hold a city model or a topology, which
/// can't be written back as they were read.
pub fn is_syncable(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".shp", ".geojson"].iter().any(|e| lower.ends_with(e))
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::{FileStamp, FileSync};
    use std::path::{Path, PathBuf};

    /// The files of the source at `path`, each as a name beside it and its path: for a
    /// Shapefile the companions its reader opens (the `.cpg` in whichever case is there, else
    /// `.cpg`), otherwise the file alone.
    fn parts(path: &Path) -> Vec<(String, PathBuf)> {
        let mut paths = vec![path.to_path_buf()];
        if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("shp")) {
            for ext in ["shx", "dbf", "prj"] {
                paths.push(path.with_extension(ext));
            }
            let cpg = crate::dbf::CPG_SPELLINGS
                .iter()
                .map(|e| path.with_extension(e))
                .find(|p| p.is_file())
                .unwrap_or_else(|| path.with_extension("cpg"));
            paths.push(cpg);
        }
        paths.into_iter().map(|p| (p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), p)).collect()
    }

    /// How recent a modification time is too recent to trust: a file system that keeps times
    /// to the second or two (FAT, some network shares) can give a write made just after it
    /// the same time.
    const FRESH: std::time::Duration = std::time::Duration::from_secs(3);

    /// The file as it is now, hashed, or `None` when it isn't there. Its time is left out
    /// when it is too recent to tell a later write by ([`FRESH`]), so that the file is always
    /// hashed when compared.
    fn stamp(path: &Path) -> std::io::Result<Option<FileStamp>> {
        let m = match std::fs::metadata(path) {
            Ok(m) if m.is_file() => m,
            Ok(_) => return Err(std::io::Error::other(format!("{} isn't a file", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let modified = m.modified().ok().filter(|t| t.elapsed().is_ok_and(|age| age >= FRESH));
        let modified = modified.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).and_then(|d| u64::try_from(d.as_nanos()).ok());
        Ok(Some(FileStamp { len: m.len(), modified, sha256: sha256(path)? }))
    }

    /// The SHA-256 of the file's bytes, read a block at a time.
    fn sha256(path: &Path) -> std::io::Result<String> {
        use sha2::Digest;
        use std::io::Read;
        let mut file = std::fs::File::open(path)?;
        let mut hasher = sha2::Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(buf.get(..n).unwrap_or_default());
        }
        Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
    }

    impl FileSync {
        /// The files of the source at `path` as they are now, each read whole to hash it.
        pub fn read(path: &Path) -> std::io::Result<FileSync> {
            let files = parts(path).into_iter().map(|(name, p)| Ok((name, stamp(&p)?))).collect::<std::io::Result<_>>()?;
            Ok(FileSync { files })
        }

        /// Runs `read`, which reads the source at `path`, and returns with its result the files
        /// as it read them: hashed before and after, so a write while it ran is caught whatever
        /// the file times say. `None` when they can't be read or were written to: what was read
        /// may then be a mix, and there is nothing to compare with later.
        pub fn around<R>(path: &Path, read: impl FnOnce() -> R) -> (R, Option<FileSync>) {
            let before = FileSync::read(path).ok();
            let out = read();
            let content =
                |s: &FileSync| s.files.iter().map(|(n, s)| (n.clone(), s.as_ref().map(|s| (s.len, s.sha256.clone())))).collect::<Vec<_>>();
            let sync = before.and_then(|before| FileSync::read(path).ok().filter(|after| content(after) == content(&before)));
            (out, sync)
        }

        /// The names of the files that differ now from this record: changed, added, removed or
        /// unreadable. Empty when the source is as it was. A file whose size and recorded
        /// modification time are both unchanged is taken as unchanged without reading it; any
        /// other of the same size is hashed, so a touch or a copy isn't counted as a change.
        pub fn changed(&self, path: &Path) -> Vec<String> {
            let now = parts(path);
            let mut changed = Vec::new();
            // Files the set has now that the record lacks (a `.cpg` in another case).
            for (name, _) in &now {
                if !self.files.iter().any(|(n, _)| n == name) {
                    changed.push(name.clone());
                }
            }
            for (name, was) in &self.files {
                let Some((_, p)) = now.iter().find(|(n, _)| n == name) else {
                    // Out of the set now: changed if it held anything.
                    if was.is_some() {
                        changed.push(name.clone());
                    }
                    continue;
                };
                let same = match (was, std::fs::metadata(p)) {
                    (None, Err(e)) => e.kind() == std::io::ErrorKind::NotFound,
                    (Some(was), Ok(m)) if m.is_file() && m.len() == was.len => {
                        let modified = m
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .and_then(|d| u64::try_from(d.as_nanos()).ok());
                        (was.modified.is_some() && modified == was.modified) || sha256(p).is_ok_and(|h| h == was.sha256)
                    }
                    _ => false,
                };
                if !same {
                    changed.push(name.clone());
                }
            }
            changed
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn a_shapefile_set_records_its_companions_and_the_missing_ones() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        for ext in ["shp", "shx", "dbf"] {
            std::fs::write(d.join(format!("a.{ext}")), ext).unwrap();
        }
        std::fs::write(d.join("a.cpg"), "UTF-8").unwrap();
        let path = d.join("a.shp");
        let sync = FileSync::read(&path).unwrap();
        let names: Vec<(&str, bool)> = sync.files.iter().map(|(n, s)| (n.as_str(), s.is_some())).collect();
        assert_eq!(names, [("a.shp", true), ("a.shx", true), ("a.dbf", true), ("a.prj", false), ("a.cpg", true)]);
        assert!(sync.changed(&path).is_empty());
    }

    #[test]
    fn changes_are_named_and_a_touch_is_not_one() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let path = d.join("a.shp");
        for ext in ["shp", "shx", "dbf"] {
            std::fs::write(d.join(format!("a.{ext}")), ext).unwrap();
        }
        let sync = FileSync::read(&path).unwrap();
        // The same bytes written again (a new time): read and hashed, not a change.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(d.join("a.dbf"), "dbf").unwrap();
        assert!(sync.changed(&path).is_empty());
        // Other bytes of the same length, a file added and one removed.
        std::fs::write(d.join("a.dbf"), "DBF").unwrap();
        std::fs::write(d.join("a.prj"), "GEOGCS").unwrap();
        std::fs::remove_file(d.join("a.shx")).unwrap();
        assert_eq!(sync.changed(&path), ["a.shx", "a.dbf", "a.prj"]);
    }

    #[test]
    fn a_write_while_reading_leaves_no_record() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let path = d.join("a.geojson");
        std::fs::write(&path, "{}").unwrap();
        let ((), sync) = FileSync::around(&path, || std::fs::write(&path, "{ }").unwrap());
        assert_eq!(sync, None);
        let ((), sync) = FileSync::around(&path, || ());
        assert_eq!(sync.map(|s| s.files.len()), Some(1));
    }

    #[test]
    fn a_record_round_trips_without_an_unknown_time() {
        let sync = FileSync { files: vec![("a.geojson".into(), Some(FileStamp { len: 2, modified: None, sha256: "ab".into() }))] };
        let text = serde_json::to_string(&sync).unwrap();
        assert_eq!(text, r#"{"files":[["a.geojson",{"len":2,"sha256":"ab"}]]}"#);
        assert_eq!(serde_json::from_str::<FileSync>(&text).unwrap(), sync);
    }
}
