//! Putting a commit's tree in a folder, and taking the last one out again.
//!
//! Only the difference is written. A conversion materialises every commit in
//! stream order, which walks branches back and forth, and rewriting every file
//! each time would cost the whole repository per commit.

use std::fs;
use std::path::{Path, PathBuf};

use historica::store::STORE_DIR;

use super::Error;
use crate::stream::Mode;
use crate::tree::{Held, Tree};

/// Refuse a target that holds anything, so that a conversion can never remove
/// something a person put there.
///
/// The only paths [`materialise`] deletes are ones an earlier commit of the
/// same conversion wrote, and this is what makes that true from the start.
pub fn must_be_free(folder: &Path) -> Result<(), Error> {
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        // Absent is free, and `from_stream` creates it.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(Error::io(folder, error)),
    };
    // The first entry is enough: this is a refusal, not an inventory.
    match entries.into_iter().next() {
        None => Ok(()),
        Some(entry) => {
            let entry = entry.map_err(|error| Error::io(folder, error))?;
            Err(Error::NotEmpty {
                folder: folder.to_path_buf(),
                holding: entry.file_name().to_string_lossy().into_owned(),
            })
        }
    }
}

/// Make the folder hold `target`, given that it holds `previous`.
pub fn materialise(folder: &Path, previous: &Tree, target: &Tree) -> Result<(), Error> {
    // Gone first, so that a file becoming a directory — or the other way
    // round — does not meet itself on the way in.
    for (path, _) in previous.iter() {
        if target.get(path).is_none() {
            let full = join(folder, path)?;
            remove(&full)?;
            prune(folder, &full)?;
        }
    }
    for (path, entry) in target.iter() {
        if previous.get(path) == Some(entry) {
            continue;
        }
        let full = join(folder, path)?;
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
        }
        // Whatever is there is not what should be there, and a link cannot be
        // written over in place.
        remove(&full)?;
        match (&entry.content, entry.mode) {
            (Held::Bytes(bytes), Mode::Symlink) => {
                let target = String::from_utf8_lossy(bytes).into_owned();
                symlink(&target, &full)?;
            }
            (Held::Bytes(bytes), mode) => {
                fs::write(&full, bytes).map_err(|error| Error::io(&full, error))?;
                set_mode(&full, matches!(mode, Mode::Executable))?;
            }
            // A submodule is another repository's commit. There is nothing to
            // write, and `Report::uncarried` is where it is said.
            (Held::Submodule(_), _) => {}
        }
    }
    Ok(())
}

/// A path as historica spells one: UTF-8, relative, and not the store's.
pub fn path_of(path: &[u8]) -> Result<String, Error> {
    let text = std::str::from_utf8(path).map_err(|_| Error::PathNotText {
        path: String::from_utf8_lossy(path).into_owned(),
    })?;
    let refuse = |because: &str| {
        Err(Error::UnusablePath {
            path: text.to_owned(),
            because: because.to_owned(),
        })
    };
    if text.is_empty() {
        return refuse("it is empty");
    }
    if text.starts_with('/') {
        return refuse("it is absolute, and a repository holds relative paths");
    }
    if text.split('/').any(|part| part == ".." || part == ".") {
        return refuse("it climbs out of the folder it is in");
    }
    // The store lives at `history/` inside the folder being converted into, so
    // a repository with its own `history/` would have its files written into
    // the store — or, worse, over it.
    if text == STORE_DIR || text.starts_with(&format!("{STORE_DIR}/")) {
        return refuse(
            "`history/` is where the store goes, so a repository that has one of \
             its own cannot be converted into a folder this way",
        );
    }
    Ok(text.to_owned())
}

fn join(folder: &Path, path: &[u8]) -> Result<PathBuf, Error> {
    Ok(folder.join(path_of(path)?))
}

fn remove(full: &Path) -> Result<(), Error> {
    // `symlink_metadata`, so that a link to a directory is removed as a link.
    match fs::symlink_metadata(full) {
        Ok(found) if found.is_dir() => {
            fs::remove_dir_all(full).map_err(|error| Error::io(full, error))
        }
        Ok(_) => fs::remove_file(full).map_err(|error| Error::io(full, error)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::io(full, error)),
    }
}

/// Remove the directories a deletion emptied, up to but never including the
/// folder itself. Git has no empty directories, so a folder that kept one
/// would not be the tree the commit describes.
fn prune(folder: &Path, from: &Path) -> Result<(), Error> {
    let mut at = from.parent();
    while let Some(directory) = at {
        if directory == folder || !directory.starts_with(folder) {
            return Ok(());
        }
        match fs::remove_dir(directory) {
            Ok(()) => at = directory.parent(),
            // Not empty, or already gone: either way there is nothing above it
            // to prune.
            Err(_) => return Ok(()),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn symlink(target: &str, at: &Path) -> Result<(), Error> {
    std::os::unix::fs::symlink(target, at).map_err(|error| Error::io(at, error))
}

#[cfg(unix)]
fn set_mode(at: &Path, executable: bool) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if executable { 0o755 } else { 0o644 };
    fs::set_permissions(at, fs::Permissions::from_mode(mode)).map_err(|error| Error::io(at, error))
}

#[cfg(not(unix))]
fn symlink(_target: &str, at: &Path) -> Result<(), Error> {
    Err(Error::NoLinks {
        path: at.to_path_buf(),
    })
}

#[cfg(not(unix))]
fn set_mode(_at: &Path, _executable: bool) -> Result<(), Error> {
    // Decision 0034 in historica reads a mode from the filesystem. Where there
    // is no mode to set, a file is recorded as the ordinary one, which is what
    // the folder will report.
    Ok(())
}
