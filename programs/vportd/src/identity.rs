// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

use std::fmt;

use vport_proto::MAX_IDENTITY_BYTES;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Identity {
    Local { uid: u32 },
    Remote { name: String },
}

impl fmt::Display for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local { uid } => write!(f, "uid {uid}"),
            Self::Remote { name } => write!(f, "remote {name}"),
        }
    }
}

pub fn valid_remote_name(name: &str) -> bool {
    !name.is_empty()
    && name.len() <= MAX_IDENTITY_BYTES
    && name.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_names() {
        assert!(valid_remote_name("totem-12.store@acme"));
    }

    #[test]
    fn rejects_empty_spaces_and_controls() {
        assert!(!valid_remote_name(""));
        assert!(!valid_remote_name("a b"));
        assert!(!valid_remote_name("a\nb"));
        assert!(!valid_remote_name("é"));
    }

    #[test]
    fn rejects_long_names() {
        assert!(valid_remote_name(&"a".repeat(MAX_IDENTITY_BYTES)));
        assert!(!valid_remote_name(&"a".repeat(MAX_IDENTITY_BYTES + 1)));
    }
}
