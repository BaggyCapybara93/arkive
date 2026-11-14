use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

use super::error::protocol as invalid;
use crate::error::AppError;

pub(super) const VERSION: u32 = 1;
pub(super) const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAX_MANIFEST_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAX_HELLO_BYTES: usize = 4 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Action {
    Check,
    Push,
    List,
    Copy,
    Download,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Message {
    Hello {
        version: u32,
        token: String,
        action: Action,
    },
    Ready {
        vault_id: String,
    },
    Push {
        snapshot_id: String,
        manifest_size: usize,
    },
    Missing {
        objects: Vec<(String, u64)>,
    },
    SnapshotComplete {
        snapshot_id: String,
    },
    List {
        path: String,
    },
    Listing {
        entries: Vec<RemoteEntry>,
    },
    Copy {
        destination: String,
        manifest_size: usize,
    },
    Download {
        source: String,
    },
    FileManifest {
        manifest_size: usize,
    },
    TransferReady,
    FileComplete {
        bytes: u64,
    },
    Error {
        message: String,
    },
}

#[derive(Serialize, Deserialize)]
pub(super) struct RemoteEntry {
    pub name: String,
    pub kind: String,
    pub size: u64,
}

pub(super) fn write(stream: &mut impl Write, message: &Message) -> Result<(), AppError> {
    let bytes = serde_json::to_vec(message)
        .map_err(|error| invalid(format!("Could not encode remote message: {error}")))?;
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(invalid("Remote message is too large"));
    }
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)?;
    stream.flush()?;
    Ok(())
}

pub(super) fn read_limited(stream: &mut impl Read, limit: usize) -> Result<Message, AppError> {
    let mut length = [0u8; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > limit {
        return Err(invalid("Invalid remote message length"));
    }
    let mut bytes = vec![0u8; length];
    stream.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes)
        .map_err(|error| invalid(format!("Invalid remote message: {error}")))
}

pub(super) fn read(stream: &mut impl Read) -> Result<Message, AppError> {
    read_limited(stream, MAX_MESSAGE_BYTES)
}
