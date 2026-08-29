//! The questions a conversion asks git besides the stream.
//!
//! Decision 0002 makes the fast-import stream the whole of this crate's contact
//! with git's objects, and says in the same breath that some facts need a
//! second question — `cat-file`, `rev-parse` — because the stream does not
//! carry them. This is where those questions are asked. Each is git's own
//! plumbing run as a program and read as text; no git object is written here,
//! and none is read except through git.

use std::collections::{BTreeMap, BTreeSet};
use std::error;
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};
use std::rc::Rc;

use crate::stream::Mode;
use crate::tree::{Entry, Held, Tree};

/// A git repository, by the directory a person would name.
#[derive(Clone, Debug)]
pub struct Repository {
    at: PathBuf,
}

/// Where a ref points, once anything it points *through* has been followed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Pointed {
    /// The commit reached.
    pub commit: String,
    /// Whether the ref named a tag object rather than the commit itself — an
    /// annotated tag, which decision 0006 does not follow into a bookmark.
    pub annotated: bool,
}

impl Repository {
    /// The repository at `at`, which must hold `.git`.
    ///
    /// Tested by the directory rather than by asking git, because git answers
    /// for any directory *inside* a repository, and a target that happens to
    /// sit under somebody's checkout is not that checkout.
    pub fn at(at: &Path) -> Option<Self> {
        at.join(".git").exists().then(|| Repository {
            at: at.to_path_buf(),
        })
    }

    /// Where the repository is.
    pub fn path(&self) -> &Path {
        &self.at
    }

    /// The directory git keeps its own files in — `.git/`, resolved, which is
    /// where a worktree's or a `gitdir:` file's answer differs from the name.
    pub fn git_dir(&self) -> Result<PathBuf, Error> {
        let out = self.ask(&["rev-parse", "--absolute-git-dir"], &[])?;
        let text = String::from_utf8_lossy(&out).trim().to_owned();
        if text.is_empty() {
            return Err(Error::Unreadable {
                command: "rev-parse --absolute-git-dir".to_owned(),
                because: "git named no directory".to_owned(),
            });
        }
        Ok(PathBuf::from(text))
    }

    /// Every ref and the commit it reaches, peeled: a tag that is an object of
    /// its own answers with the commit it points at, and says that it did.
    pub fn refs(&self) -> Result<BTreeMap<String, Pointed>, Error> {
        let out = self.ask(
            &[
                "for-each-ref",
                "--format=%(objecttype) %(if)%(*objectname)%(then)%(*objectname)%(else)%(objectname)%(end) %(refname)",
            ],
            &[],
        )?;
        let mut refs = BTreeMap::new();
        for line in String::from_utf8_lossy(&out).lines() {
            let mut words = line.splitn(3, ' ');
            let (Some(kind), Some(oid), Some(name)) = (words.next(), words.next(), words.next())
            else {
                continue;
            };
            refs.insert(
                name.to_owned(),
                Pointed {
                    commit: oid.to_owned(),
                    annotated: kind == "tag",
                },
            );
        }
        Ok(refs)
    }

    /// Which of these object IDs the repository actually holds.
    ///
    /// A remembered correspondence can outlive the objects it names — a
    /// repository is a thing people run `gc` in — so a conversion checks before
    /// it names a commit it did not send.
    pub fn holding<'a>(
        &self,
        oids: impl IntoIterator<Item = &'a str>,
    ) -> Result<BTreeSet<String>, Error> {
        let mut asked = String::new();
        for oid in oids {
            asked.push_str(oid);
            asked.push('\n');
        }
        if asked.is_empty() {
            return Ok(BTreeSet::new());
        }
        let out = self.ask(&["cat-file", "--batch-check"], asked.as_bytes())?;
        let mut held = BTreeSet::new();
        for line in String::from_utf8_lossy(&out).lines() {
            let mut words = line.split(' ');
            if let (Some(oid), Some(kind)) = (words.next(), words.next())
                && kind == "commit"
            {
                held.insert(oid.to_owned());
            }
        }
        Ok(held)
    }

    /// The tree at a commit, read from git rather than replayed.
    ///
    /// An incremental export names the commits it left out by object ID, and a
    /// commit standing on one is stated as a difference against it — so the
    /// replay needs that tree, and git is the only party that has it exactly.
    pub fn tree_of(&self, oid: &str) -> Result<Tree, Error> {
        let listed = self.ask(&["ls-tree", "-r", "-z", oid], &[])?;
        let mut entries: Vec<(Mode, String, Vec<u8>)> = Vec::new();
        for record in listed.split(|byte| *byte == 0) {
            if record.is_empty() {
                continue;
            }
            let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
                return Err(self.unreadable("ls-tree", "an entry had no path"));
            };
            let (head, path) = (&record[..tab], &record[tab + 1..]);
            let head = String::from_utf8_lossy(head);
            let mut words = head.split(' ');
            let (Some(mode), Some(_kind), Some(object)) =
                (words.next(), words.next(), words.next())
            else {
                return Err(self.unreadable("ls-tree", "an entry was not mode, type, object"));
            };
            let mode = match mode {
                "100644" => Mode::File,
                "100755" => Mode::Executable,
                "120000" => Mode::Symlink,
                "160000" => Mode::Gitlink,
                other => {
                    return Err(self.unreadable("ls-tree", &format!("mode `{other}` at a file")));
                }
            };
            entries.push((mode, object.to_owned(), path.to_vec()));
        }

        // Every blob in one question. `--batch` answers each with a header line
        // and then the bytes, in the order asked.
        let mut asked = String::new();
        for (mode, object, _) in &entries {
            if *mode != Mode::Gitlink {
                asked.push_str(object);
                asked.push('\n');
            }
        }
        let mut blobs: BTreeMap<String, Rc<[u8]>> = BTreeMap::new();
        if !asked.is_empty() {
            let out = self.ask(&["cat-file", "--batch"], asked.as_bytes())?;
            let mut at = 0;
            while at < out.len() {
                let Some(end) = out[at..].iter().position(|byte| *byte == b'\n') else {
                    break;
                };
                let header = String::from_utf8_lossy(&out[at..at + end]).into_owned();
                at += end + 1;
                let mut words = header.split(' ');
                let (Some(object), Some(kind), size) = (words.next(), words.next(), words.next())
                else {
                    return Err(
                        self.unreadable("cat-file --batch", "a header was not what it says")
                    );
                };
                if kind == "missing" {
                    return Err(self.unreadable(
                        "cat-file --batch",
                        &format!("blob {object} is not in the repository"),
                    ));
                }
                let size: usize = size.and_then(|size| size.parse().ok()).ok_or_else(|| {
                    self.unreadable("cat-file --batch", "a header stated no size")
                })?;
                if at + size > out.len() {
                    return Err(self.unreadable("cat-file --batch", "the output ended early"));
                }
                blobs.insert(object.to_owned(), Rc::from(&out[at..at + size]));
                // The bytes, and then the newline git puts after them.
                at += size + 1;
            }
        }

        let mut tree = Tree::default();
        for (mode, object, path) in entries {
            let content = match mode {
                Mode::Gitlink => Held::Submodule(object),
                _ => Held::Bytes(blobs.get(&object).map(Rc::clone).ok_or_else(|| {
                    self.unreadable(
                        "cat-file --batch",
                        &format!("blob {object} was not answered"),
                    )
                })?),
            };
            tree.insert(path, Entry { mode, content });
        }
        Ok(tree)
    }

    /// The parents of a commit, in the order git states them.
    pub fn parents_of(&self, oid: &str) -> Result<Vec<String>, Error> {
        let out = self.ask(&["rev-list", "--parents", "-n", "1", oid], &[])?;
        Ok(String::from_utf8_lossy(&out)
            .split_whitespace()
            .skip(1)
            .map(str::to_owned)
            .collect())
    }

    /// The object ID that names nothing, at the width this repository's
    /// object format spells one — forty zeros under SHA-1, sixty-four under
    /// SHA-256.
    pub fn null_oid(&self) -> Result<String, Error> {
        let out = self.ask(&["rev-parse", "--show-object-format"], &[])?;
        let width = match String::from_utf8_lossy(&out).trim() {
            "sha256" => 64,
            _ => 40,
        };
        Ok("0".repeat(width))
    }

    /// The branch HEAD names, if it names one.
    pub fn head(&self) -> Result<Option<String>, Error> {
        let out = Process::new("git")
            .arg("-C")
            .arg(&self.at)
            .args(["symbolic-ref", "--quiet", "HEAD"])
            .stderr(Stdio::null())
            .output()
            .map_err(Error::Spawn)?;
        if !out.status.success() {
            return Ok(None);
        }
        let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        Ok((!text.is_empty()).then_some(text))
    }

    /// Whether the index and the working tree agree with HEAD. Untracked files
    /// are not asked about: they are somebody's and `reset --hard` leaves them.
    pub fn is_clean(&self) -> Result<bool, Error> {
        let out = self.ask(&["status", "--porcelain", "--untracked-files=no"], &[])?;
        Ok(out.iter().all(|byte| byte.is_ascii_whitespace()))
    }

    /// Keep the store directory out of git without writing a tracked file.
    /// Existing local exclusions are preserved and the managed line is added
    /// at most once.
    pub fn exclude_history(&self) -> Result<(), Error> {
        let out = self.ask(&["rev-parse", "--git-path", "info/exclude"], &[])?;
        let named = PathBuf::from(String::from_utf8_lossy(&out).trim());
        let path = if named.is_absolute() {
            named
        } else {
            self.at.join(named)
        };
        let mut contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(Error::Io { path, error }),
        };
        if contents.lines().any(|line| line.trim() == "/history/") {
            return Ok(());
        }
        if !contents.is_empty() && !contents.ends_with('\n') {
            contents.push('\n');
        }
        contents.push_str("/history/\n");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| Error::Io {
                path: parent.to_path_buf(),
                error,
            })?;
        }
        fs::write(&path, contents).map_err(|error| Error::Io { path, error })
    }

    /// Bring git's index to HEAD without touching the working tree.
    pub fn reset_index(&self) -> Result<(), Error> {
        self.run(&["reset", "--quiet", "--mixed"])
    }

    /// Run a git command for its effect, and fail if it did.
    pub fn run(&self, arguments: &[&str]) -> Result<(), Error> {
        self.ask(arguments, &[]).map(|_| ())
    }

    /// Run a git command with `input` on its stdin and hand back its stdout.
    fn ask(&self, arguments: &[&str], input: &[u8]) -> Result<Vec<u8>, Error> {
        let mut child = Process::new("git")
            .arg("-C")
            .arg(&self.at)
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(Error::Spawn)?;

        // Written on its own thread. The output of `cat-file --batch` can be
        // larger than a pipe, and a process that blocks writing it while this
        // one blocks feeding it is a conversion that never finishes.
        let mut stdin = child.stdin.take().expect("stdin was piped");
        let feeding = input.to_vec();
        let writing = std::thread::spawn(move || stdin.write_all(&feeding));

        let mut complaints = child.stderr.take().expect("stderr was piped");
        let listening = std::thread::spawn(move || {
            let mut said = Vec::new();
            let _ = complaints.read_to_end(&mut said);
            said
        });

        let mut out = Vec::new();
        let mut stdout = child.stdout.take().expect("stdout was piped");
        let read = stdout.read_to_end(&mut out);

        let _ = writing.join();
        let said = listening.join().unwrap_or_default();
        // Waited for before the read is judged, so that a read that failed
        // does not leave a child behind.
        let status = child.wait().map_err(Error::Spawn)?;
        read.map_err(Error::Spawn)?;
        if !status.success() {
            return Err(Error::Failed {
                command: arguments.join(" "),
                status: status.code(),
                said: String::from_utf8_lossy(&said)
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .map(str::to_owned)
                    .collect(),
            });
        }
        Ok(out)
    }

    fn unreadable(&self, command: &str, because: &str) -> Error {
        Error::Unreadable {
            command: command.to_owned(),
            because: because.to_owned(),
        }
    }
}

/// A question git would not answer.
#[derive(Debug)]
pub enum Error {
    /// `git` could not be run at all.
    Spawn(io::Error),
    /// Git's local administrative files could not be read or written.
    Io {
        /// The administrative file being accessed.
        path: PathBuf,
        /// What the filesystem reported.
        error: io::Error,
    },
    /// git ran and refused.
    Failed {
        /// What was asked.
        command: String,
        /// Its exit code, where it had one.
        status: Option<i32>,
        /// What it said on the way out.
        said: Vec<String>,
    },
    /// git answered, and the answer was not in the shape its manual promises.
    Unreadable {
        /// What was asked.
        command: String,
        /// What was wrong with the answer.
        because: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Spawn(error) => write!(
                f,
                "`git` could not be run: {error} — decision 0002 makes it this \
                 crate's one dependency, and it has to be on PATH"
            ),
            Error::Io { path, error } => write!(f, "{}: {error}", path.display()),
            Error::Failed {
                command,
                status,
                said,
            } => {
                match status {
                    Some(code) => write!(f, "`git {command}` exited {code}")?,
                    None => write!(f, "`git {command}` was stopped")?,
                }
                for line in said {
                    write!(f, "\n  {line}")?;
                }
                Ok(())
            }
            Error::Unreadable { command, because } => {
                write!(
                    f,
                    "`git {command}` answered something unexpected: {because}"
                )
            }
        }
    }
}

impl error::Error for Error {}
