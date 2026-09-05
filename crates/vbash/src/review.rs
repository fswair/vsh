use std::collections::BTreeSet;

use vsh_policy::{AccessKind, CallPolicy};
use vsh_store::{BlobStore, BlobStoreError};
use vsh_types::{BlobId, ContentVersion, DiffEntry, NodeKind, NodeState, VPath};
use vsh_vfs::{Effect, EffectEvent};

use crate::ReviewContent;

pub(crate) fn collect_content(
    changes: &[DiffEntry],
    effects: &[EffectEvent],
    policy: &CallPolicy,
    store: &BlobStore,
    maximum: usize,
) -> Result<(Vec<ReviewContent>, bool), BlobStoreError> {
    if maximum == 0 {
        let needs_content = changes.iter().any(|entry| {
            [entry.before, entry.after]
                .into_iter()
                .flatten()
                .any(|state| state.kind() != NodeKind::Directory)
        }) || effects
            .iter()
            .any(|event| matches!(event.effect, Effect::ContentRead { .. }));
        return Ok((Vec::new(), !needs_content));
    }
    let mut collector = ContentCollector {
        policy,
        store,
        remaining: maximum,
        complete: true,
        seen: BTreeSet::new(),
        contents: Vec::new(),
    };
    for change in changes {
        for state in [change.before, change.after].into_iter().flatten() {
            collector.node(&change.path, state)?;
        }
    }
    for event in effects {
        if let Effect::ContentRead { path, blob } = &event.effect {
            collector.blob(path, *blob)?;
        }
    }
    Ok((collector.contents, collector.complete))
}

struct ContentCollector<'a> {
    policy: &'a CallPolicy,
    store: &'a BlobStore,
    remaining: usize,
    complete: bool,
    seen: BTreeSet<(VPath, BlobId)>,
    contents: Vec<ReviewContent>,
}

impl ContentCollector<'_> {
    fn node(&mut self, path: &VPath, state: NodeState) -> Result<(), BlobStoreError> {
        if state.kind() != NodeKind::Directory {
            if let Some(ContentVersion::Blob(blob)) = state.content() {
                self.blob(path, blob)?;
            } else {
                self.complete = false;
            }
        }
        Ok(())
    }

    fn blob(&mut self, path: &VPath, blob: BlobId) -> Result<(), BlobStoreError> {
        if !self.seen.insert((path.clone(), blob)) {
            return Ok(());
        }
        if self
            .policy
            .authorize(path, AccessKind::ContentRead)
            .is_err()
        {
            self.complete = false;
            return Ok(());
        }
        let bytes = match self.store.get_bounded(blob, self.remaining) {
            Ok(bytes) => bytes,
            Err(BlobStoreError::SizeLimit { .. }) => {
                self.complete = false;
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        self.remaining -= bytes.len();
        self.contents.push(ReviewContent {
            path: path.clone(),
            blob,
            bytes,
        });
        Ok(())
    }
}
