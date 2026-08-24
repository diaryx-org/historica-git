//! Identifiers that come from the commit rather than from entropy.
//!
//! Decision 0003: a change ID is the first 96 bits of the git commit's object
//! ID, so that the same repository converted twice is the same history rather
//! than two of them. File identifiers are derived from the same object ID,
//! because they are written into the revision's bytes and a random one would
//! undo the whole point.
//!
//! [`Identity`] is what [`historica::record::Entropy`] wants: one commit's
//! worth of identifiers, handed to `record` in place of the platform's random
//! source.

use std::error;
use std::fmt;

use historica::core::{CHANGE_ID_LEN, ChangeId};
use historica::record::{Entropy, SourceError};
use sha2::{Digest, Sha256};

use crate::stream::Commit;

/// The identifiers one commit converts to.
#[derive(Clone, Debug)]
pub struct Identity {
    /// The commit's object ID, as bytes.
    oid: Vec<u8>,
    /// How many identifiers have been drawn, so that two files minted under one
    /// commit do not collide.
    drawn: u64,
}

impl Identity {
    /// The identifiers for a commit the stream carried an object ID for.
    ///
    /// A stream written without `--show-original-ids` carries none, and is
    /// refused here rather than quietly minted around: a conversion that fell
    /// back to entropy would produce a store that looks right and converges
    /// with nothing.
    pub fn of(commit: &Commit) -> Result<Self, Error> {
        let Some(oid) = &commit.original_oid else {
            return Err(Error::NoObjectId);
        };
        Self::from_oid(oid)
    }

    /// The identifiers for an object ID, spelled in hex as git spells it.
    pub fn from_oid(oid: &str) -> Result<Self, Error> {
        let bytes = unhex(oid).ok_or_else(|| Error::NotAnObjectId {
            oid: oid.to_owned(),
        })?;
        if bytes.len() < CHANGE_ID_LEN {
            return Err(Error::TooShort {
                oid: oid.to_owned(),
                bytes: bytes.len(),
            });
        }
        Ok(Identity {
            oid: bytes,
            drawn: 0,
        })
    }

    /// The change this commit is, which does not depend on how many identifiers
    /// have been drawn — [`Entropy::change`] may be called at any point, and
    /// twice for one commit must answer twice the same.
    pub fn change_id(&self) -> ChangeId {
        let mut bytes = [0u8; CHANGE_ID_LEN];
        bytes.copy_from_slice(&self.oid[..CHANGE_ID_LEN]);
        ChangeId::from_bytes(bytes)
    }
}

impl Entropy for Identity {
    /// Bytes nobody chose, but everybody can recompute: `SHA-256` of the object
    /// ID and a counter, which is a derivation a person can check with the
    /// `shasum` they already have.
    ///
    /// Only file identifiers reach this — [`Entropy::change`] is overridden
    /// below — and they are drawn in the order historica asks for them, which
    /// decision 0003 records as the one coupling this design has.
    fn fill(&mut self, bytes: &mut [u8]) -> Result<(), SourceError> {
        let mut at = 0;
        while at < bytes.len() {
            let mut hasher = Sha256::new();
            hasher.update(&self.oid);
            hasher.update(self.drawn.to_be_bytes());
            let block = hasher.finalize();
            self.drawn += 1;
            let taking = block.len().min(bytes.len() - at);
            bytes[at..at + taking].copy_from_slice(&block[..taking]);
            at += taking;
        }
        Ok(())
    }

    fn change(&mut self) -> Result<ChangeId, SourceError> {
        Ok(self.change_id())
    }
}

/// An object ID a conversion cannot use.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Error {
    /// The stream carried no object ID for this commit.
    NoObjectId,
    /// What the stream carried was not hex.
    NotAnObjectId {
        /// What it said.
        oid: String,
    },
    /// Hex, but too little of it to name a change.
    TooShort {
        /// What it said.
        oid: String,
        /// How many bytes that came to.
        bytes: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoObjectId => write!(
                f,
                "this commit carries no object ID — the export must be written \
                 with `--show-original-ids`, which is what decision 0003 derives \
                 a change from"
            ),
            Error::NotAnObjectId { oid } => {
                write!(f, "`{oid}` is not an object ID: it is not hex")
            }
            Error::TooShort { oid, bytes } => write!(
                f,
                "`{oid}` is {bytes} bytes, and a change ID needs {CHANGE_ID_LEN}"
            ),
        }
    }
}

impl error::Error for Error {}

/// Hex to bytes, or `None` if it is not hex. An odd number of digits is not an
/// object ID: git writes every byte as two.
fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.is_empty() || !text.len().is_multiple_of(2) {
        return None;
    }
    // The length is already even, so the remainder `as_chunks` returns is
    // empty.
    let (pairs, _) = text.as_bytes().as_chunks::<2>();
    let mut bytes = Vec::with_capacity(pairs.len());
    for pair in pairs {
        let high = nibble(pair[0])?;
        let low = nibble(pair[1])?;
        bytes.push((high << 4) | low);
    }
    Some(bytes)
}

fn nibble(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        // Git writes lowercase, and a stream that shouted would be a stream
        // some other tool wrote. Accepting it costs nothing and refusing it
        // would be a rule with no reason behind it.
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use historica::core::{FILE_ID_LEN, FileId};

    use super::*;

    /// The commit `make.sh` builds first, and the change it converts to. The
    /// spelling is decision 0003's worked example, and pins the whole scheme:
    /// reversed hex over the object ID's own first twelve bytes.
    const OID: &str = "980aeeabdf5024a43392620b11f1d14de03a0bb5";
    const CHANGE: &str = "qrzpllpomkuzxvpvwwqxtxzo";

    #[test]
    fn a_change_is_the_commit_it_came_from() {
        let identity = Identity::from_oid(OID).expect("a real object ID");
        assert_eq!(identity.change_id().to_string(), CHANGE);
    }

    #[test]
    fn every_nibble_maps_the_way_the_decision_says() {
        // 0 through f, twice over, so each of the sixteen letters is pinned.
        let identity = Identity::from_oid("0123456789abcdef0123456789abcdef").expect("hex is hex");
        assert_eq!(identity.change_id().to_string(), "zyxwvutsrqponmlkzyxwvuts");
    }

    #[test]
    fn the_change_does_not_move_as_files_are_drawn() {
        let mut identity = Identity::from_oid(OID).expect("a real object ID");
        let before = identity.change().expect("a change");
        let _ = identity.file().expect("a file");
        let _ = identity.file().expect("another file");
        let after = identity.change().expect("the same change");
        assert_eq!(before, after);
    }

    #[test]
    fn two_conversions_of_one_commit_agree_throughout() {
        let draw = || {
            let mut identity = Identity::from_oid(OID).expect("a real object ID");
            let change = identity.change().expect("a change");
            let files: Vec<_> = (0..4).map(|_| identity.file().expect("a file")).collect();
            (change, files)
        };
        assert_eq!(draw(), draw());
    }

    #[test]
    fn files_minted_under_one_commit_differ() {
        let mut identity = Identity::from_oid(OID).expect("a real object ID");
        let files: Vec<FileId> = (0..8).map(|_| identity.file().expect("a file")).collect();
        let mut unique = files.clone();
        unique.sort_by_key(|file| file.to_string());
        unique.dedup_by_key(|file| file.to_string());
        assert_eq!(unique.len(), files.len(), "a file ID was drawn twice");
        assert_eq!(FILE_ID_LEN, CHANGE_ID_LEN, "both are 96 bits");
    }

    #[test]
    fn two_commits_do_not_share_their_files() {
        let mut one = Identity::from_oid(OID).expect("a real object ID");
        let mut other =
            Identity::from_oid("00112233445566778899aabbccddeeff00112233").expect("hex is hex");
        assert_ne!(
            one.file().expect("a file").to_string(),
            other.file().expect("a file").to_string()
        );
    }

    #[test]
    fn what_is_refused_says_why() {
        assert_eq!(
            Identity::from_oid("nothex!!").err(),
            Some(Error::NotAnObjectId {
                oid: "nothex!!".to_owned()
            })
        );
        assert!(matches!(
            Identity::from_oid("abcd"),
            Err(Error::TooShort { .. })
        ));
        assert!(
            Error::NoObjectId
                .to_string()
                .contains("--show-original-ids"),
            "the message has to name the flag that would fix it"
        );
    }
}
