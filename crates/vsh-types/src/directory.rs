use crate::{ContentVersion, DirectoryDigest, NodeState, VPath, domain_hasher};

pub(super) fn digest_entries<'a, I>(entries: I) -> DirectoryDigest
where
    I: Iterator<Item = (&'a VPath, NodeState)> + Clone,
{
    // directory-v1 prefixes the payload length. Replay immutable observations to
    // determine that length without retaining all paths or their serialization.
    let payload_bytes = entries.clone().fold(0_u64, |total, (path, state)| {
        let state_bytes = match state.content() {
            None => 14,
            Some(ContentVersion::Blob(_)) => 46,
            Some(ContentVersion::Stamp(stamp)) => 60 + u64::from(stamp.ctime_ns.is_some()) * 16,
        };
        total
            .checked_add(8 + path.as_str().len() as u64 + state_bytes)
            .expect("an addressable canonical directory listing fits u64")
    });
    let mut sink = DirectoryHasher {
        hasher: domain_hasher(b"directory-v1", payload_bytes),
        buffer: [0; 4096],
        buffered: 0,
    };
    // The maximum current node encoding is a 14-byte header + 62-byte stamp.
    // Allocate only when a nonempty listing actually needs a state encoding.
    let mut state_bytes = Vec::new();
    for (path, state) in entries {
        sink.update(&(path.as_str().len() as u64).to_le_bytes());
        sink.update(path.as_str().as_bytes());
        state_bytes.clear();
        state.encode_canonical(&mut state_bytes);
        sink.update(&state_bytes);
    }
    sink.hasher.update(&sink.buffer[..sink.buffered]);
    DirectoryDigest::from_bytes(*sink.hasher.finalize().as_bytes())
}

struct DirectoryHasher {
    hasher: blake3::Hasher,
    buffer: [u8; 4096],
    buffered: usize,
}

impl DirectoryHasher {
    fn update(&mut self, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            if self.buffered == 0 && bytes.len() >= self.buffer.len() {
                self.hasher.update(bytes);
                return;
            }
            let copied = bytes.len().min(self.buffer.len() - self.buffered);
            self.buffer[self.buffered..self.buffered + copied].copy_from_slice(&bytes[..copied]);
            self.buffered += copied;
            bytes = &bytes[copied..];
            if self.buffered == self.buffer.len() {
                self.hasher.update(&self.buffer);
                self.buffered = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{BlobId, FileStamp, NodeKind, PlatformFileId};

    use super::*;

    #[test]
    fn streaming_listing_matches_v1_encoding_across_states_and_buffer_edges() {
        let stamp = FileStamp {
            kind: NodeKind::File,
            size: 17,
            mode: 0o640,
            mtime_ns: -7,
            ctime_ns: Some(13),
            file_id: PlatformFileId { high: 3, low: 5 },
        };
        let states = [
            NodeState::directory(0o755),
            NodeState::file(BlobId::from_bytes([0x5a; 32]), 123, 0o600),
            NodeState::symlink(BlobId::from_bytes([0xa5; 32]), 99, 0o777),
            NodeState::from_stamp(stamp),
            NodeState::from_stamp(FileStamp {
                ctime_ns: None,
                ..stamp
            }),
        ];
        for length in [0, 1, 37, 4095, 4096, 4097, 16384] {
            let paths: Vec<_> = (0..states.len())
                .map(|index| VPath::parse(&format!("{index}é{}", "x".repeat(length))).unwrap())
                .collect();
            for count in 0..=states.len() {
                let entries = paths[..count].iter().zip(states[..count].iter().copied());
                let mut canonical = Vec::new();
                for (path, state) in entries.clone() {
                    canonical.extend_from_slice(&(path.as_str().len() as u64).to_le_bytes());
                    canonical.extend_from_slice(path.as_str().as_bytes());
                    state.encode_canonical(&mut canonical);
                }
                assert_eq!(
                    DirectoryDigest::digest_entries(entries),
                    DirectoryDigest::digest_canonical(&canonical)
                );
            }
        }
    }
}
