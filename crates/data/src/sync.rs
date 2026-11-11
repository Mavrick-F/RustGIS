//! The files a layer was read from, as they were when it was read ([`FileSync`]): what a layer
//! kept in the project compares with its source before its edits are written back to it, so
//! that writing them never overwrites a change made to the file since (#1385).

use serde::{Deserialize, Serialize};

/// The files of a source as they were at a moment: each one's name, size and SHA-256 hash, and
/// the files that weren't there. A Shapefile is its `.shp`, `.shx`, `.dbf`, `.prj` and `.cpg`
/// (the ones its reader opens); a GeoJSON file is itself. Only the contents count: a file
/// copied, touched or moved to another disk with the same bytes is the same file.
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
    use crate::{DataError, Result};
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

    /// The file's size, or `None` when it isn't there.
    fn len(path: &Path) -> Result<Option<u64>> {
        match std::fs::metadata(path) {
            Ok(m) if m.is_file() => Ok(Some(m.len())),
            Ok(_) => Err(DataError::Format(format!("{} isn't a file", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// The SHA-256 of the file's bytes, read a block at a time, each counted as progress
    /// ([`crate::progress`]) and stopping once the run is cancelled.
    fn sha256(path: &Path) -> Result<String> {
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
            crate::progress::step(n)?;
        }
        Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
    }

    /// Bytes as a progress count.
    fn count(bytes: u64) -> usize {
        usize::try_from(bytes).unwrap_or(usize::MAX)
    }

    impl FileSync {
        /// The files of the source at `path` as they are now, each read whole to hash it.
        pub fn read(path: &Path) -> Result<FileSync> {
            let parts = parts(path);
            let total = parts.iter().filter_map(|(_, p)| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
            crate::progress::begin(count(total));
            let mut files = Vec::new();
            for (name, p) in parts {
                let stamp = match len(&p)? {
                    None => None,
                    Some(len) => Some(FileStamp { len, sha256: sha256(&p)? }),
                };
                files.push((name, stamp));
            }
            Ok(FileSync { files })
        }

        /// Runs `read`, which reads the source at `path`, and returns with its result the files
        /// as it read them: hashed before and after, so a write while it ran is caught. `None`
        /// when they can't be read or were written to: what was read may then be a mix, and
        /// there is nothing to compare with later.
        pub fn around<R>(path: &Path, read: impl FnOnce() -> R) -> (R, Option<FileSync>) {
            let before = FileSync::read(path).ok();
            let out = read();
            let sync = before.and_then(|before| FileSync::read(path).ok().filter(|after| *after == before));
            (out, sync)
        }

        /// The names of the files that differ now from this record: changed, added, removed or
        /// unreadable. Empty when the source is as it was. A file of the size recorded is read
        /// whole and hashed, so its work grows with the file: a caller on the desktop's UI
        /// thread does it on a thread of its own. Errs only when the run is cancelled.
        pub fn changed(&self, path: &Path) -> Result<Vec<String>> {
            let now = parts(path);
            let mut changed = Vec::new();
            // Files there now that the record lacks (a `.cpg` added in another case); not the
            // `.cpg` name `parts` falls back to when there is none.
            for (name, p) in &now {
                if !self.files.iter().any(|(n, _)| n == name) && p.exists() {
                    changed.push(name.clone());
                }
            }
            let total = self.files.iter().filter_map(|(_, s)| s.as_ref()).map(|s| s.len).sum();
            crate::progress::begin(count(total));
            for (name, was) in &self.files {
                let Some((_, p)) = now.iter().find(|(n, _)| n == name) else {
                    // Out of the set now: changed if it held anything.
                    if was.is_some() {
                        changed.push(name.clone());
                    }
                    continue;
                };
                let same = match (was, len(p)) {
                    (None, Ok(None)) => true,
                    (Some(was), Ok(Some(len))) if len == was.len => match sha256(p) {
                        Ok(hash) => hash == was.sha256,
                        Err(DataError::Cancelled) => return Err(DataError::Cancelled),
                        Err(_) => false,
                    },
                    _ => false,
                };
                if !same {
                    changed.push(name.clone());
                }
            }
            Ok(changed)
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
        assert!(sync.changed(&path).unwrap().is_empty());
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
        // The same bytes written again: not a change.
        std::fs::write(d.join("a.dbf"), "dbf").unwrap();
        assert!(sync.changed(&path).unwrap().is_empty());
        // Other bytes of the same length, a file added and one removed.
        std::fs::write(d.join("a.dbf"), "DBF").unwrap();
        std::fs::write(d.join("a.prj"), "GEOGCS").unwrap();
        std::fs::remove_file(d.join("a.shx")).unwrap();
        assert_eq!(sync.changed(&path).unwrap(), ["a.shx", "a.dbf", "a.prj"]);
    }

    /// A copy that keeps times (`cp -p`, `rsync -t`) can put back the size and time a file had
    /// with other bytes: the hash still tells.
    #[test]
    fn a_rewrite_that_puts_the_time_back_is_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.geojson");
        std::fs::write(&path, "{}").unwrap();
        let sync = FileSync::read(&path).unwrap();
        let time = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::write(&path, "[]").unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(time).unwrap();
        assert_eq!(sync.changed(&path).unwrap(), ["a.geojson"]);
    }

    #[test]
    fn a_removed_cpg_is_named_as_it_was_spelled() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        for ext in ["shp", "shx", "dbf"] {
            std::fs::write(d.join(format!("a.{ext}")), ext).unwrap();
        }
        // Recorded under a spelling no file has now: as `a.CPG` is once removed on a
        // case-sensitive file system (one that ignores case would find `a.cpg` there).
        let path = d.join("a.shp");
        let mut sync = FileSync::read(&path).unwrap();
        let cpg = sync.files.iter_mut().find(|(n, _)| n == "a.cpg").unwrap();
        *cpg = ("a.CPG".into(), Some(FileStamp { len: 5, sha256: "00".into() }));
        assert_eq!(sync.changed(&path).unwrap(), ["a.CPG"]);
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
    fn a_record_round_trips() {
        let sync = FileSync { files: vec![("a.geojson".into(), Some(FileStamp { len: 2, sha256: "ab".into() })), ("a.prj".into(), None)] };
        let text = serde_json::to_string(&sync).unwrap();
        assert_eq!(text, r#"{"files":[["a.geojson",{"len":2,"sha256":"ab"}],["a.prj",null]]}"#);
        assert_eq!(serde_json::from_str::<FileSync>(&text).unwrap(), sync);
    }
}
