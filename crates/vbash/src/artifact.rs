use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use vsh_monty::ExecutionStats;
use vsh_policy::{RiskFlag, RiskManifest, RiskMetrics, read_set_digest, write_set_digest};
use vsh_types::{
    ContentVersion, DiffDigest, DiffEntry, DiffKind, DirectoryDigest, FileStamp, IntentDigest,
    NodeKind, NodeState, PlatformFileId, PolicyDigest, ProgramDigest, ReadSetDigest,
    RuntimeConfigDigest, SnapshotId, TransactionBinding, VPath, WriteSetDigest,
};
use vsh_vfs::{
    CanonicalDiff, Effect, EffectEvent, EffectOrigin, ReadObservation, WritePrecondition,
};

use crate::runtime::{ArtifactLimits, Receipt, RuntimeDecision, StageTimings};

const ARTIFACT_MAGIC_V1: &[u8; 8] = b"VSHPND01";
const ARTIFACT_MAGIC_V2: &[u8; 8] = b"VSHPND02";

#[derive(Clone)]
pub(crate) struct ReviewEvidence {
    pub(crate) intent: Option<String>,
    pub(crate) metrics: RiskMetrics,
    pub(crate) effects: Vec<EffectEvent>,
    pub(crate) complete: bool,
    pub(crate) truncated: bool,
}

#[derive(Clone)]
pub(crate) struct PendingTransaction {
    pub(crate) binding: TransactionBinding,
    pub(crate) diff: CanonicalDiff,
    pub(crate) read_set: BTreeMap<VPath, ReadObservation>,
    pub(crate) write_set: BTreeMap<VPath, WritePrecondition>,
    pub(crate) review: ReviewEvidence,
    pub(crate) receipt: Receipt,
}

pub(crate) fn encode_pending(
    artifact: &PendingTransaction,
    limits: ArtifactLimits,
) -> Result<Vec<u8>, ArtifactError> {
    let mut output = Encoder::new(limits.max_bytes);
    output.extend_from_slice(ARTIFACT_MAGIC_V2)?;
    encode_binding(&artifact.binding, &mut output)?;
    encode_review_evidence(&artifact.review, limits, &mut output)?;
    output.push(u8::from(!artifact.receipt.changes.is_empty()))?;
    encode_decision(&artifact.receipt.decision, &mut output)?;

    let value_len =
        postcard::experimental::serialized_size(&artifact.receipt.value).map_err(|source| {
            ArtifactError::ValueCodec {
                operation: "size",
                detail: source.to_string(),
            }
        })?;
    if value_len > limits.max_value_bytes {
        return Err(ArtifactError::Limit {
            field: "result value",
            observed: value_len,
            maximum: limits.max_value_bytes,
        });
    }
    let mut value = Vec::new();
    value
        .try_reserve_exact(value_len)
        .map_err(|source| ArtifactError::Allocation {
            field: "result value",
            requested: value_len,
            detail: source.to_string(),
        })?;
    value.resize(value_len, 0);
    postcard::to_slice(&artifact.receipt.value, value.as_mut_slice()).map_err(|source| {
        ArtifactError::ValueCodec {
            operation: "encode",
            detail: source.to_string(),
        }
    })?;
    encode_bytes(&value, &mut output)?;
    if artifact.receipt.stdout.len() > limits.max_stdout_bytes {
        return Err(ArtifactError::Limit {
            field: "stdout",
            observed: artifact.receipt.stdout.len(),
            maximum: limits.max_stdout_bytes,
        });
    }
    encode_bytes(artifact.receipt.stdout.as_bytes(), &mut output)?;
    encode_execution_stats(artifact.receipt.execution, &mut output)?;
    encode_timings(artifact.receipt.timings, &mut output)?;

    if artifact.diff.entries().len() > limits.max_entries {
        return Err(ArtifactError::Limit {
            field: "diff entries",
            observed: artifact.diff.entries().len(),
            maximum: limits.max_entries,
        });
    }
    encode_len(artifact.diff.entries().len(), &mut output)?;
    for entry in artifact.diff.entries() {
        encode_path(&entry.path, limits, &mut output)?;
        encode_optional_state(entry.before, &mut output)?;
        encode_optional_state(entry.after, &mut output)?;
        output.push(diff_kind_tag(entry.kind))?;
    }

    if artifact.read_set.len() > limits.max_dependencies {
        return Err(ArtifactError::Limit {
            field: "read dependencies",
            observed: artifact.read_set.len(),
            maximum: limits.max_dependencies,
        });
    }
    encode_len(artifact.read_set.len(), &mut output)?;
    for (path, observation) in &artifact.read_set {
        encode_path(path, limits, &mut output)?;
        match observation.metadata {
            None => output.push(0)?,
            Some(None) => output.push(1)?,
            Some(Some(state)) => {
                output.push(2)?;
                encode_state(state, &mut output)?;
            }
        }
        encode_optional_digest(
            observation.content.map(|value| *value.as_bytes()),
            &mut output,
        )?;
        encode_optional_digest(
            observation.directory.map(|value| *value.as_bytes()),
            &mut output,
        )?;
    }

    if artifact.write_set.len() > limits.max_dependencies {
        return Err(ArtifactError::Limit {
            field: "write dependencies",
            observed: artifact.write_set.len(),
            maximum: limits.max_dependencies,
        });
    }
    encode_len(artifact.write_set.len(), &mut output)?;
    for (path, precondition) in &artifact.write_set {
        encode_path(path, limits, &mut output)?;
        encode_optional_state(precondition.expected, &mut output)?;
    }

    Ok(output.finish())
}

pub(crate) fn decode_pending(
    bytes: &[u8],
    limits: ArtifactLimits,
) -> Result<PendingTransaction, ArtifactError> {
    if bytes.len() > limits.max_bytes {
        return Err(ArtifactError::Limit {
            field: "pending artifact",
            observed: bytes.len(),
            maximum: limits.max_bytes,
        });
    }
    let mut decoder = Decoder::new(bytes);
    let magic = decoder.take(ARTIFACT_MAGIC_V2.len())?;
    let has_review_evidence = if magic == ARTIFACT_MAGIC_V2 {
        true
    } else if magic == ARTIFACT_MAGIC_V1 {
        false
    } else {
        return Err(decoder.corrupt("invalid pending-artifact header"));
    };
    let binding = decode_binding(&mut decoder)?;
    let review = has_review_evidence
        .then(|| decode_review_evidence(&mut decoder, limits))
        .transpose()?;
    let full_detail = match decoder.byte()? {
        0 => false,
        1 => true,
        _ => return Err(decoder.corrupt("invalid receipt-detail tag")),
    };
    let decision = decode_decision(&mut decoder)?;
    let value_bytes = decoder.length_prefixed(limits.max_value_bytes, "result value")?;
    let value = postcard::from_bytes(value_bytes).map_err(|source| ArtifactError::ValueCodec {
        operation: "decode",
        detail: source.to_string(),
    })?;
    let stdout = decoder.string(limits.max_stdout_bytes, "stdout")?;
    let execution = decode_execution_stats(&mut decoder)?;
    let timings = decode_timings(&mut decoder)?;

    let diff = decode_diff(&mut decoder, limits)?;
    let read_set = decode_read_set(&mut decoder, limits)?;
    let write_set = decode_write_set(&mut decoder, limits)?;
    decoder.finish()?;

    if binding.diff != diff.digest()
        || binding.read_set != read_set_digest(&read_set)
        || binding.write_set != write_set_digest(&write_set)
    {
        return Err(ArtifactError::BindingMismatch);
    }
    let state = match &decision {
        RuntimeDecision::AutoApproved => vsh_types::TransactionState::AutoApproved,
        RuntimeDecision::PendingApproval(_) => vsh_types::TransactionState::PendingApproval,
        RuntimeDecision::Denied(_) => return Err(decoder.corrupt("denied artifact is pending")),
    };
    let review = review.unwrap_or_else(|| ReviewEvidence {
        intent: None,
        metrics: match &decision {
            RuntimeDecision::PendingApproval(manifest) => manifest.metrics,
            RuntimeDecision::AutoApproved | RuntimeDecision::Denied(_) => RiskMetrics::default(),
        },
        effects: Vec::new(),
        complete: false,
        truncated: false,
    });
    let changes = if full_detail {
        diff.entries().to_vec()
    } else {
        Vec::new()
    };
    let receipt = Receipt {
        transaction: binding.transaction_id(),
        base_snapshot: binding.base_snapshot,
        state,
        decision,
        diff: diff.digest(),
        changed_paths: diff.entries().len(),
        changes,
        value,
        stdout,
        execution,
        timings,
        commit: None,
    };
    Ok(PendingTransaction {
        binding,
        diff,
        read_set,
        write_set,
        review,
        receipt,
    })
}

fn encode_review_evidence(
    review: &ReviewEvidence,
    limits: ArtifactLimits,
    output: &mut Encoder,
) -> Result<(), ArtifactError> {
    match &review.intent {
        None => output.push(0)?,
        Some(intent) => {
            if intent.len() > limits.max_intent_bytes {
                return Err(ArtifactError::Limit {
                    field: "intent",
                    observed: intent.len(),
                    maximum: limits.max_intent_bytes,
                });
            }
            output.push(1)?;
            encode_bytes(intent.as_bytes(), output)?;
        }
    }
    encode_risk_metrics(review.metrics, output)?;
    if review.effects.len() > limits.max_effects {
        return Err(ArtifactError::Limit {
            field: "review effects",
            observed: review.effects.len(),
            maximum: limits.max_effects,
        });
    }
    encode_len(review.effects.len(), output)?;
    for event in &review.effects {
        output.extend_from_slice(&event.sequence.to_le_bytes())?;
        output.push(effect_origin_tag(event.origin)?)?;
        encode_effect(&event.effect, limits, output)?;
    }
    output.push(u8::from(review.complete))?;
    output.push(u8::from(review.truncated))
}

fn decode_review_evidence(
    decoder: &mut Decoder<'_>,
    limits: ArtifactLimits,
) -> Result<ReviewEvidence, ArtifactError> {
    let intent = match decoder.byte()? {
        0 => None,
        1 => Some(decoder.string(limits.max_intent_bytes, "intent")?),
        _ => return Err(decoder.corrupt("unknown optional-intent tag")),
    };
    let metrics = decode_risk_metrics(decoder)?;
    let count = decoder.length(limits.max_effects, "review effects")?;
    let mut effects = Vec::with_capacity(count);
    for _ in 0..count {
        let sequence = decoder.u64()?;
        let origin = decode_effect_origin(decoder.byte()?)
            .ok_or_else(|| decoder.corrupt("unknown effect-origin tag"))?;
        let effect = decode_effect(decoder, limits)?;
        effects.push(EffectEvent {
            sequence,
            origin,
            effect,
        });
    }
    if !effects
        .windows(2)
        .all(|pair| pair[0].sequence < pair[1].sequence)
    {
        return Err(decoder.corrupt("effect sequences are not strictly increasing"));
    }
    let complete = decode_bool(decoder, "evidence-complete")?;
    let truncated = decode_bool(decoder, "evidence-truncated")?;
    Ok(ReviewEvidence {
        intent,
        metrics,
        effects,
        complete,
        truncated,
    })
}

fn encode_effect(
    effect: &Effect,
    limits: ArtifactLimits,
    output: &mut Encoder,
) -> Result<(), ArtifactError> {
    match effect {
        Effect::MetadataRead { path, state } => {
            output.push(1)?;
            encode_path(path, limits, output)?;
            encode_optional_state(*state, output)?;
        }
        Effect::ContentRead { path, blob } => {
            output.push(2)?;
            encode_path(path, limits, output)?;
            output.extend_from_slice(blob.as_bytes())?;
        }
        Effect::DirectoryRead { path, digest } => {
            output.push(3)?;
            encode_path(path, limits, output)?;
            output.extend_from_slice(digest.as_bytes())?;
        }
        Effect::Create { path, after } => {
            output.push(4)?;
            encode_path(path, limits, output)?;
            encode_state(*after, output)?;
        }
        Effect::ModifyContent {
            path,
            before,
            after,
        } => {
            output.push(5)?;
            encode_path(path, limits, output)?;
            encode_state(*before, output)?;
            encode_state(*after, output)?;
        }
        Effect::Delete { path, before } => {
            output.push(6)?;
            encode_path(path, limits, output)?;
            encode_state(*before, output)?;
        }
        Effect::Rename {
            from,
            to,
            before,
            after,
        } => {
            output.push(7)?;
            encode_path(from, limits, output)?;
            encode_path(to, limits, output)?;
            encode_state(*before, output)?;
            encode_state(*after, output)?;
        }
        _ => {
            return Err(ArtifactError::Unsupported {
                reason: "unknown effect variant",
            });
        }
    }
    Ok(())
}

fn decode_effect(
    decoder: &mut Decoder<'_>,
    limits: ArtifactLimits,
) -> Result<Effect, ArtifactError> {
    match decoder.byte()? {
        1 => Ok(Effect::MetadataRead {
            path: decoder.path(limits)?,
            state: decode_optional_state(decoder)?,
        }),
        2 => Ok(Effect::ContentRead {
            path: decoder.path(limits)?,
            blob: vsh_types::BlobId::from_bytes(decoder.digest()?),
        }),
        3 => Ok(Effect::DirectoryRead {
            path: decoder.path(limits)?,
            digest: DirectoryDigest::from_bytes(decoder.digest()?),
        }),
        4 => Ok(Effect::Create {
            path: decoder.path(limits)?,
            after: decode_state(decoder)?,
        }),
        5 => Ok(Effect::ModifyContent {
            path: decoder.path(limits)?,
            before: decode_state(decoder)?,
            after: decode_state(decoder)?,
        }),
        6 => Ok(Effect::Delete {
            path: decoder.path(limits)?,
            before: decode_state(decoder)?,
        }),
        7 => Ok(Effect::Rename {
            from: decoder.path(limits)?,
            to: decoder.path(limits)?,
            before: decode_state(decoder)?,
            after: decode_state(decoder)?,
        }),
        _ => Err(decoder.corrupt("unknown effect tag")),
    }
}

fn effect_origin_tag(origin: EffectOrigin) -> Result<u8, ArtifactError> {
    match origin {
        EffectOrigin::VirtualFs => Ok(1),
        EffectOrigin::MontyOsCall => Ok(2),
        EffectOrigin::MontyToolCall => Ok(3),
        _ => Err(ArtifactError::Unsupported {
            reason: "unknown effect origin",
        }),
    }
}

const fn decode_effect_origin(tag: u8) -> Option<EffectOrigin> {
    match tag {
        1 => Some(EffectOrigin::VirtualFs),
        2 => Some(EffectOrigin::MontyOsCall),
        3 => Some(EffectOrigin::MontyToolCall),
        _ => None,
    }
}

fn decode_bool(decoder: &mut Decoder<'_>, field: &'static str) -> Result<bool, ArtifactError> {
    match decoder.byte()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(decoder.corrupt(field)),
    }
}

fn encode_binding(binding: &TransactionBinding, output: &mut Encoder) -> Result<(), ArtifactError> {
    output.extend_from_slice(binding.base_snapshot.as_bytes())?;
    output.extend_from_slice(binding.diff.as_bytes())?;
    output.extend_from_slice(binding.read_set.as_bytes())?;
    output.extend_from_slice(binding.write_set.as_bytes())?;
    output.extend_from_slice(binding.program.as_bytes())?;
    output.extend_from_slice(binding.policy.as_bytes())?;
    output.extend_from_slice(binding.runtime_config.as_bytes())?;
    encode_optional_digest(binding.intent.map(|value| *value.as_bytes()), output)
}

fn decode_diff(
    decoder: &mut Decoder<'_>,
    limits: ArtifactLimits,
) -> Result<CanonicalDiff, ArtifactError> {
    let entry_count = decoder.length(limits.max_entries, "diff entries")?;
    let mut entries = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        let path = decoder.path(limits)?;
        let before = decode_optional_state(decoder)?;
        let after = decode_optional_state(decoder)?;
        let kind = decode_diff_kind(decoder.byte()?)
            .ok_or_else(|| decoder.corrupt("unknown diff-kind tag"))?;
        entries.push(DiffEntry {
            path,
            before,
            after,
            kind,
        });
    }
    CanonicalDiff::from_entries(entries)
        .map_err(|_| decoder.corrupt("decoded diff is not canonical"))
}

fn decode_read_set(
    decoder: &mut Decoder<'_>,
    limits: ArtifactLimits,
) -> Result<BTreeMap<VPath, ReadObservation>, ArtifactError> {
    let count = decoder.length(limits.max_dependencies, "read dependencies")?;
    let mut read_set = BTreeMap::new();
    for _ in 0..count {
        let path = decoder.path(limits)?;
        let metadata = match decoder.byte()? {
            0 => None,
            1 => Some(None),
            2 => Some(Some(decode_state(decoder)?)),
            _ => return Err(decoder.corrupt("unknown metadata-observation tag")),
        };
        let content = decode_optional_digest(decoder)?.map(vsh_types::BlobId::from_bytes);
        let directory = decode_optional_digest(decoder)?.map(DirectoryDigest::from_bytes);
        if read_set
            .insert(
                path,
                ReadObservation {
                    metadata,
                    content,
                    directory,
                },
            )
            .is_some()
        {
            return Err(decoder.corrupt("duplicate read dependency"));
        }
    }
    Ok(read_set)
}

fn decode_write_set(
    decoder: &mut Decoder<'_>,
    limits: ArtifactLimits,
) -> Result<BTreeMap<VPath, WritePrecondition>, ArtifactError> {
    let count = decoder.length(limits.max_dependencies, "write dependencies")?;
    let mut write_set = BTreeMap::new();
    for _ in 0..count {
        let path = decoder.path(limits)?;
        let expected = decode_optional_state(decoder)?;
        if write_set
            .insert(path, WritePrecondition { expected })
            .is_some()
        {
            return Err(decoder.corrupt("duplicate write dependency"));
        }
    }
    Ok(write_set)
}

fn decode_binding(decoder: &mut Decoder<'_>) -> Result<TransactionBinding, ArtifactError> {
    Ok(TransactionBinding {
        base_snapshot: SnapshotId::from_bytes(decoder.digest()?),
        diff: DiffDigest::from_bytes(decoder.digest()?),
        read_set: ReadSetDigest::from_bytes(decoder.digest()?),
        write_set: WriteSetDigest::from_bytes(decoder.digest()?),
        program: ProgramDigest::from_bytes(decoder.digest()?),
        policy: PolicyDigest::from_bytes(decoder.digest()?),
        runtime_config: RuntimeConfigDigest::from_bytes(decoder.digest()?),
        intent: decode_optional_digest(decoder)?.map(IntentDigest::from_bytes),
    })
}

fn encode_decision(decision: &RuntimeDecision, output: &mut Encoder) -> Result<(), ArtifactError> {
    match decision {
        RuntimeDecision::AutoApproved => output.push(1)?,
        RuntimeDecision::PendingApproval(manifest) => {
            output.push(2)?;
            encode_risk_manifest(manifest, output)?;
        }
        RuntimeDecision::Denied(_) => {
            return Err(ArtifactError::Unsupported {
                reason: "denied transactions cannot be pending",
            });
        }
    }
    Ok(())
}

fn decode_decision(decoder: &mut Decoder<'_>) -> Result<RuntimeDecision, ArtifactError> {
    match decoder.byte()? {
        1 => Ok(RuntimeDecision::AutoApproved),
        2 => decode_risk_manifest(decoder).map(RuntimeDecision::PendingApproval),
        _ => Err(decoder.corrupt("unknown runtime-decision tag")),
    }
}

fn encode_risk_manifest(
    manifest: &RiskManifest,
    output: &mut Encoder,
) -> Result<(), ArtifactError> {
    encode_risk_metrics(manifest.metrics, output)?;
    encode_len(manifest.flags.len(), output)?;
    for flag in &manifest.flags {
        output.push(risk_flag_tag(*flag))?;
    }
    output.extend_from_slice(manifest.policy.as_bytes())?;
    Ok(())
}

fn decode_risk_manifest(decoder: &mut Decoder<'_>) -> Result<RiskManifest, ArtifactError> {
    let metrics = decode_risk_metrics(decoder)?;
    let count = decoder.length(32, "risk flags")?;
    let mut flags = Vec::with_capacity(count);
    for _ in 0..count {
        flags.push(
            decode_risk_flag(decoder.byte()?)
                .ok_or_else(|| decoder.corrupt("unknown risk-flag tag"))?,
        );
    }
    if !flags.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(decoder.corrupt("risk flags are not strictly ordered"));
    }
    Ok(RiskManifest {
        metrics,
        flags,
        policy: PolicyDigest::from_bytes(decoder.digest()?),
    })
}

fn encode_risk_metrics(metrics: RiskMetrics, output: &mut Encoder) -> Result<(), ArtifactError> {
    encode_len(metrics.touched_paths, output)?;
    encode_len(metrics.created_paths, output)?;
    encode_len(metrics.modified_paths, output)?;
    encode_len(metrics.deleted_paths, output)?;
    encode_len(metrics.renamed_paths, output)?;
    output.extend_from_slice(&metrics.changed_bytes.to_le_bytes())?;
    output.extend_from_slice(&metrics.delete_ratio_bps.to_le_bytes())?;
    encode_len(metrics.executable_changes, output)?;
    encode_len(metrics.symlink_changes, output)
}

fn decode_risk_metrics(decoder: &mut Decoder<'_>) -> Result<RiskMetrics, ArtifactError> {
    Ok(RiskMetrics {
        touched_paths: decoder.usize()?,
        created_paths: decoder.usize()?,
        modified_paths: decoder.usize()?,
        deleted_paths: decoder.usize()?,
        renamed_paths: decoder.usize()?,
        changed_bytes: decoder.u64()?,
        delete_ratio_bps: decoder.u16()?,
        executable_changes: decoder.usize()?,
        symlink_changes: decoder.usize()?,
    })
}

fn encode_execution_stats(
    stats: ExecutionStats,
    output: &mut Encoder,
) -> Result<(), ArtifactError> {
    output.extend_from_slice(&stats.os_calls.to_le_bytes())?;
    output.extend_from_slice(&stats.read_bytes.to_le_bytes())?;
    output.extend_from_slice(&stats.write_bytes.to_le_bytes())?;
    output.extend_from_slice(&stats.directory_entries.to_le_bytes())?;
    encode_len(stats.output_bytes, output)?;
    output.extend_from_slice(&stats.denied_accesses.to_le_bytes())?;
    output.extend_from_slice(&stats.result_bytes.to_le_bytes())?;
    Ok(())
}

fn decode_execution_stats(decoder: &mut Decoder<'_>) -> Result<ExecutionStats, ArtifactError> {
    Ok(ExecutionStats {
        os_calls: decoder.u64()?,
        read_bytes: decoder.u64()?,
        write_bytes: decoder.u64()?,
        directory_entries: decoder.u64()?,
        output_bytes: decoder.usize()?,
        denied_accesses: decoder.u64()?,
        result_bytes: decoder.u64()?,
    })
}

fn encode_timings(timings: StageTimings, output: &mut Encoder) -> Result<(), ArtifactError> {
    output.extend_from_slice(&timings.snapshot_ns.to_le_bytes())?;
    output.extend_from_slice(&timings.execute_ns.to_le_bytes())?;
    output.extend_from_slice(&timings.diff_ns.to_le_bytes())?;
    output.extend_from_slice(&timings.policy_ns.to_le_bytes())?;
    output.extend_from_slice(&timings.bind_and_store_ns.to_le_bytes())?;
    output.extend_from_slice(&timings.commit_ns.to_le_bytes())?;
    output.extend_from_slice(&timings.total_ns.to_le_bytes())
}

fn decode_timings(decoder: &mut Decoder<'_>) -> Result<StageTimings, ArtifactError> {
    Ok(StageTimings {
        snapshot_ns: decoder.u64()?,
        execute_ns: decoder.u64()?,
        diff_ns: decoder.u64()?,
        policy_ns: decoder.u64()?,
        bind_and_store_ns: decoder.u64()?,
        commit_ns: decoder.u64()?,
        total_ns: decoder.u64()?,
    })
}

fn encode_path(
    path: &VPath,
    limits: ArtifactLimits,
    output: &mut Encoder,
) -> Result<(), ArtifactError> {
    if path.as_str().len() > limits.max_path_bytes {
        return Err(ArtifactError::Limit {
            field: "path",
            observed: path.as_str().len(),
            maximum: limits.max_path_bytes,
        });
    }
    encode_bytes(path.as_str().as_bytes(), output)
}

fn encode_optional_state(
    state: Option<NodeState>,
    output: &mut Encoder,
) -> Result<(), ArtifactError> {
    match state {
        None => output.push(0)?,
        Some(state) => {
            output.push(1)?;
            encode_state(state, output)?;
        }
    }
    Ok(())
}

fn decode_optional_state(decoder: &mut Decoder<'_>) -> Result<Option<NodeState>, ArtifactError> {
    match decoder.byte()? {
        0 => Ok(None),
        1 => decode_state(decoder).map(Some),
        _ => Err(decoder.corrupt("unknown optional-state tag")),
    }
}

fn encode_state(state: NodeState, output: &mut Encoder) -> Result<(), ArtifactError> {
    output.push(node_kind_tag(state.kind()))?;
    output.extend_from_slice(&state.size().to_le_bytes())?;
    output.extend_from_slice(&state.mode().to_le_bytes())?;
    match state.content() {
        None => output.push(0)?,
        Some(ContentVersion::Blob(blob)) => {
            output.push(1)?;
            output.extend_from_slice(blob.as_bytes())?;
        }
        Some(ContentVersion::Stamp(stamp)) => {
            output.push(2)?;
            encode_stamp(stamp, output)?;
        }
        Some(_) => {
            return Err(ArtifactError::Unsupported {
                reason: "unknown node content version",
            });
        }
    }
    Ok(())
}

fn decode_state(decoder: &mut Decoder<'_>) -> Result<NodeState, ArtifactError> {
    let kind = decode_node_kind(decoder.byte()?)
        .ok_or_else(|| decoder.corrupt("unknown node-kind tag"))?;
    let size = decoder.u64()?;
    let mode = decoder.u32()?;
    let state = match decoder.byte()? {
        0 if kind == NodeKind::Directory && size == 0 => NodeState::directory(mode),
        1 if kind == NodeKind::File => {
            NodeState::file(vsh_types::BlobId::from_bytes(decoder.digest()?), size, mode)
        }
        1 if kind == NodeKind::Symlink => {
            NodeState::symlink(vsh_types::BlobId::from_bytes(decoder.digest()?), size, mode)
        }
        2 => {
            let stamp = decode_stamp(decoder)?;
            if stamp.kind != kind || stamp.size != size || stamp.mode != mode {
                return Err(decoder.corrupt("node state and metadata stamp disagree"));
            }
            NodeState::from_stamp(stamp)
        }
        _ => return Err(decoder.corrupt("invalid node content encoding")),
    };
    Ok(state)
}

fn encode_stamp(stamp: FileStamp, output: &mut Encoder) -> Result<(), ArtifactError> {
    output.push(node_kind_tag(stamp.kind))?;
    output.extend_from_slice(&stamp.size.to_le_bytes())?;
    output.extend_from_slice(&stamp.mode.to_le_bytes())?;
    output.extend_from_slice(&stamp.mtime_ns.to_le_bytes())?;
    match stamp.ctime_ns {
        None => output.push(0)?,
        Some(value) => {
            output.push(1)?;
            output.extend_from_slice(&value.to_le_bytes())?;
        }
    }
    output.extend_from_slice(&stamp.file_id.high.to_le_bytes())?;
    output.extend_from_slice(&stamp.file_id.low.to_le_bytes())
}

fn decode_stamp(decoder: &mut Decoder<'_>) -> Result<FileStamp, ArtifactError> {
    let kind = decode_node_kind(decoder.byte()?)
        .ok_or_else(|| decoder.corrupt("unknown stamp node-kind tag"))?;
    let size = decoder.u64()?;
    let mode = decoder.u32()?;
    let mtime_ns = decoder.i128()?;
    let ctime_ns = match decoder.byte()? {
        0 => None,
        1 => Some(decoder.i128()?),
        _ => return Err(decoder.corrupt("unknown optional ctime tag")),
    };
    Ok(FileStamp {
        kind,
        size,
        mode,
        mtime_ns,
        ctime_ns,
        file_id: PlatformFileId {
            high: decoder.u64()?,
            low: decoder.u64()?,
        },
    })
}

fn encode_optional_digest(
    value: Option<[u8; 32]>,
    output: &mut Encoder,
) -> Result<(), ArtifactError> {
    match value {
        None => output.push(0),
        Some(bytes) => {
            output.push(1)?;
            output.extend_from_slice(&bytes)
        }
    }
}

fn decode_optional_digest(decoder: &mut Decoder<'_>) -> Result<Option<[u8; 32]>, ArtifactError> {
    match decoder.byte()? {
        0 => Ok(None),
        1 => decoder.digest().map(Some),
        _ => Err(decoder.corrupt("unknown optional-digest tag")),
    }
}

fn encode_bytes(bytes: &[u8], output: &mut Encoder) -> Result<(), ArtifactError> {
    encode_len(bytes.len(), output)?;
    output.extend_from_slice(bytes)
}

fn encode_len(value: usize, output: &mut Encoder) -> Result<(), ArtifactError> {
    let value = u64::try_from(value).map_err(|_| ArtifactError::Unsupported {
        reason: "host length cannot be encoded",
    })?;
    output.extend_from_slice(&value.to_le_bytes())
}

const fn node_kind_tag(kind: NodeKind) -> u8 {
    match kind {
        NodeKind::File => 1,
        NodeKind::Directory => 2,
        NodeKind::Symlink => 3,
    }
}

const fn decode_node_kind(tag: u8) -> Option<NodeKind> {
    match tag {
        1 => Some(NodeKind::File),
        2 => Some(NodeKind::Directory),
        3 => Some(NodeKind::Symlink),
        _ => None,
    }
}

const fn diff_kind_tag(kind: DiffKind) -> u8 {
    match kind {
        DiffKind::Create => 1,
        DiffKind::Delete => 2,
        DiffKind::Modify => 3,
        DiffKind::MetadataChange => 4,
    }
}

const fn decode_diff_kind(tag: u8) -> Option<DiffKind> {
    match tag {
        1 => Some(DiffKind::Create),
        2 => Some(DiffKind::Delete),
        3 => Some(DiffKind::Modify),
        4 => Some(DiffKind::MetadataChange),
        _ => None,
    }
}

const fn risk_flag_tag(flag: RiskFlag) -> u8 {
    match flag {
        RiskFlag::Mutation => 1,
        RiskFlag::Deletion => 2,
        RiskFlag::Rename => 3,
        RiskFlag::ExecutableChange => 4,
        RiskFlag::SymlinkChange => 5,
        RiskFlag::LargeTouchedSet => 6,
        RiskFlag::LargeByteChange => 7,
    }
}

const fn decode_risk_flag(tag: u8) -> Option<RiskFlag> {
    match tag {
        1 => Some(RiskFlag::Mutation),
        2 => Some(RiskFlag::Deletion),
        3 => Some(RiskFlag::Rename),
        4 => Some(RiskFlag::ExecutableChange),
        5 => Some(RiskFlag::SymlinkChange),
        6 => Some(RiskFlag::LargeTouchedSet),
        7 => Some(RiskFlag::LargeByteChange),
        _ => None,
    }
}

struct Encoder {
    bytes: Vec<u8>,
    maximum: usize,
}

impl Encoder {
    const fn new(maximum: usize) -> Self {
        Self {
            bytes: Vec::new(),
            maximum,
        }
    }

    fn push(&mut self, byte: u8) -> Result<(), ArtifactError> {
        self.extend_from_slice(&[byte])
    }

    fn extend_from_slice(&mut self, bytes: &[u8]) -> Result<(), ArtifactError> {
        let required = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .ok_or(ArtifactError::Limit {
                field: "pending artifact",
                observed: usize::MAX,
                maximum: self.maximum,
            })?;
        if required > self.maximum {
            return Err(ArtifactError::Limit {
                field: "pending artifact",
                observed: required,
                maximum: self.maximum,
            });
        }
        if required > self.bytes.capacity() {
            let doubled = self.bytes.capacity().max(2_048).saturating_mul(2);
            let target = doubled.max(required).min(self.maximum);
            self.bytes
                .try_reserve_exact(target.saturating_sub(self.bytes.len()))
                .map_err(|source| ArtifactError::Allocation {
                    field: "pending artifact",
                    requested: target,
                    detail: source.to_string(),
                })?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Decoder<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], ArtifactError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or_else(|| self.corrupt("artifact offset overflow"))?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| self.corrupt("truncated pending artifact"))?;
        self.offset = end;
        Ok(value)
    }

    fn byte(&mut self) -> Result<u8, ArtifactError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, ArtifactError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, ArtifactError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, ArtifactError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn i128(&mut self) -> Result<i128, ArtifactError> {
        Ok(i128::from_le_bytes(self.array()?))
    }

    fn usize(&mut self) -> Result<usize, ArtifactError> {
        usize::try_from(self.u64()?).map_err(|_| self.corrupt("length does not fit this host"))
    }

    fn length(&mut self, maximum: usize, field: &'static str) -> Result<usize, ArtifactError> {
        let value = self.usize()?;
        if value > maximum {
            return Err(ArtifactError::Limit {
                field,
                observed: value,
                maximum,
            });
        }
        Ok(value)
    }

    fn length_prefixed(
        &mut self,
        maximum: usize,
        field: &'static str,
    ) -> Result<&'a [u8], ArtifactError> {
        let length = self.length(maximum, field)?;
        self.take(length)
    }

    fn string(&mut self, maximum: usize, field: &'static str) -> Result<String, ArtifactError> {
        let bytes = self.length_prefixed(maximum, field)?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| self.corrupt("artifact string is not UTF-8"))
    }

    fn path(&mut self, limits: ArtifactLimits) -> Result<VPath, ArtifactError> {
        let value = self.string(limits.max_path_bytes, "path")?;
        VPath::parse(&value).map_err(|_| self.corrupt("artifact path is invalid"))
    }

    fn digest(&mut self) -> Result<[u8; 32], ArtifactError> {
        self.array()
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], ArtifactError> {
        let mut output = [0_u8; N];
        output.copy_from_slice(self.take(N)?);
        Ok(output)
    }

    fn finish(&self) -> Result<(), ArtifactError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(self.corrupt("trailing pending-artifact bytes"))
        }
    }

    const fn corrupt(&self, reason: &'static str) -> ArtifactError {
        ArtifactError::Corrupt {
            offset: self.offset,
            reason,
        }
    }
}

/// Durable pending-artifact encoding or validation failure.
#[derive(Debug)]
pub enum ArtifactError {
    /// One bounded field or complete artifact exceeded configured limits.
    Limit {
        /// Bounded field.
        field: &'static str,
        /// Observed units.
        observed: usize,
        /// Maximum accepted units.
        maximum: usize,
    },
    /// A bounded allocation failed before any unbounded growth was attempted.
    Allocation {
        /// Buffer being allocated.
        field: &'static str,
        /// Requested byte capacity.
        requested: usize,
        /// Allocator failure detail.
        detail: String,
    },
    /// The content-addressed artifact is malformed.
    Corrupt {
        /// Byte offset nearest the violation.
        offset: usize,
        /// Stable validation reason.
        reason: &'static str,
    },
    /// The serialized Monty result could not be encoded or decoded.
    ValueCodec {
        /// Codec direction.
        operation: &'static str,
        /// Postcard error detail.
        detail: String,
    },
    /// A future non-exhaustive value cannot be represented safely by this codec.
    Unsupported {
        /// Stable rejection reason.
        reason: &'static str,
    },
    /// Recomputed diff/dependency identities do not match the transaction binding.
    BindingMismatch,
}

impl fmt::Display for ArtifactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limit {
                field,
                observed,
                maximum,
            } => write!(
                formatter,
                "pending artifact {field} is {observed}; maximum is {maximum}"
            ),
            Self::Allocation {
                field,
                requested,
                detail,
            } => write!(
                formatter,
                "cannot allocate {requested} bytes for pending artifact {field}: {detail}"
            ),
            Self::Corrupt { offset, reason } => {
                write!(
                    formatter,
                    "pending artifact is corrupt at byte {offset}: {reason}"
                )
            }
            Self::ValueCodec { operation, detail } => {
                write!(
                    formatter,
                    "cannot {operation} pending result value: {detail}"
                )
            }
            Self::Unsupported { reason } => {
                write!(
                    formatter,
                    "pending artifact contains an unsupported value: {reason}"
                )
            }
            Self::BindingMismatch => formatter.write_str(
                "pending artifact diff or dependencies do not match its transaction binding",
            ),
        }
    }
}

impl Error for ArtifactError {}

#[cfg(test)]
mod tests {
    use super::*;
    use vsh_monty::MontyObject;
    use vsh_types::{BlobId, TransactionState};

    fn fixture() -> PendingTransaction {
        let path = VPath::parse("result.txt").unwrap();
        let state = NodeState::file(BlobId::digest(b"result"), 6, 0o644);
        let diff = CanonicalDiff::from_entries(vec![DiffEntry {
            path: path.clone(),
            before: None,
            after: Some(state),
            kind: DiffKind::Create,
        }])
        .unwrap();
        let read_set = BTreeMap::new();
        let write_set = BTreeMap::from([(path, WritePrecondition { expected: None })]);
        let binding = TransactionBinding {
            base_snapshot: SnapshotId::from_bytes([1; 32]),
            diff: diff.digest(),
            read_set: read_set_digest(&read_set),
            write_set: write_set_digest(&write_set),
            program: ProgramDigest::digest_source("artifact-test"),
            policy: PolicyDigest::digest_canonical(b"artifact-policy"),
            runtime_config: RuntimeConfigDigest::digest_canonical(b"artifact-runtime"),
            intent: Some(IntentDigest::digest_text("create result")),
        };
        let receipt = Receipt {
            transaction: binding.transaction_id(),
            base_snapshot: binding.base_snapshot,
            state: TransactionState::AutoApproved,
            decision: RuntimeDecision::AutoApproved,
            diff: diff.digest(),
            changed_paths: 1,
            changes: diff.entries().to_vec(),
            value: MontyObject::Int(42),
            stdout: "ok\n".to_owned(),
            execution: ExecutionStats {
                os_calls: 1,
                write_bytes: 6,
                output_bytes: 3,
                result_bytes: 8,
                ..ExecutionStats::default()
            },
            timings: StageTimings {
                total_ns: 123,
                ..StageTimings::default()
            },
            commit: None,
        };
        PendingTransaction {
            binding,
            diff,
            read_set,
            write_set,
            review: ReviewEvidence {
                intent: Some("create result".to_owned()),
                metrics: RiskMetrics {
                    touched_paths: 1,
                    created_paths: 1,
                    changed_bytes: 6,
                    ..RiskMetrics::default()
                },
                effects: vec![EffectEvent {
                    sequence: 1,
                    origin: EffectOrigin::MontyOsCall,
                    effect: Effect::Create {
                        path: VPath::parse("result.txt").unwrap(),
                        after: state,
                    },
                }],
                complete: true,
                truncated: false,
            },
            receipt,
        }
    }

    #[test]
    fn pending_artifact_round_trip_preserves_exact_binding_and_receipt() {
        let artifact = fixture();
        let bytes = encode_pending(&artifact, ArtifactLimits::default()).unwrap();
        let decoded = decode_pending(&bytes, ArtifactLimits::default()).unwrap();

        assert_eq!(decoded.binding, artifact.binding);
        assert_eq!(decoded.diff, artifact.diff);
        assert_eq!(decoded.read_set, artifact.read_set);
        assert_eq!(decoded.write_set, artifact.write_set);
        assert_eq!(decoded.review.intent, artifact.review.intent);
        assert_eq!(decoded.review.metrics, artifact.review.metrics);
        assert_eq!(decoded.review.effects, artifact.review.effects);
        assert!(decoded.review.complete);
        assert!(!decoded.review.truncated);
        assert_eq!(decoded.receipt.transaction, artifact.receipt.transaction);
        assert_eq!(decoded.receipt.changes, artifact.receipt.changes);
        assert_eq!(decoded.receipt.value, MontyObject::Int(42));
        assert_eq!(decoded.receipt.stdout, "ok\n");
        assert_eq!(decoded.receipt.execution, artifact.receipt.execution);
        assert_eq!(decoded.receipt.timings, artifact.receipt.timings);
    }

    #[test]
    fn version_one_artifact_decodes_with_incomplete_review_evidence() {
        let artifact = fixture();
        let limits = ArtifactLimits::default();
        let current = encode_pending(&artifact, limits).unwrap();

        let mut binding = Encoder::new(limits.max_bytes);
        encode_binding(&artifact.binding, &mut binding).unwrap();
        let binding_len = binding.finish().len();
        let mut review = Encoder::new(limits.max_bytes);
        encode_review_evidence(&artifact.review, limits, &mut review).unwrap();
        let review_len = review.finish().len();
        let body = &current[ARTIFACT_MAGIC_V2.len()..];
        let mut legacy = Vec::with_capacity(current.len() - review_len);
        legacy.extend_from_slice(ARTIFACT_MAGIC_V1);
        legacy.extend_from_slice(&body[..binding_len]);
        legacy.extend_from_slice(&body[binding_len + review_len..]);

        let decoded = decode_pending(&legacy, limits).unwrap();
        assert_eq!(decoded.binding, artifact.binding);
        assert!(!decoded.review.complete);
        assert!(!decoded.review.truncated);
        assert!(decoded.review.intent.is_none());
        assert!(decoded.review.effects.is_empty());
    }

    #[test]
    fn pending_artifact_rejects_tampered_binding_and_trailing_bytes() {
        let mut bytes = encode_pending(&fixture(), ArtifactLimits::default()).unwrap();
        bytes[ARTIFACT_MAGIC_V2.len() + 32] ^= 0x80;
        assert!(matches!(
            decode_pending(&bytes, ArtifactLimits::default()),
            Err(ArtifactError::BindingMismatch)
        ));

        let mut bytes = encode_pending(&fixture(), ArtifactLimits::default()).unwrap();
        bytes.push(0);
        assert!(matches!(
            decode_pending(&bytes, ArtifactLimits::default()),
            Err(ArtifactError::Corrupt {
                reason: "trailing pending-artifact bytes",
                ..
            })
        ));
    }

    #[test]
    fn pending_artifact_limits_apply_before_unbounded_materialization() {
        let artifact = fixture();
        let limits = ArtifactLimits {
            max_value_bytes: 0,
            ..ArtifactLimits::default()
        };
        assert!(matches!(
            encode_pending(&artifact, limits),
            Err(ArtifactError::Limit {
                field: "result value",
                maximum: 0,
                ..
            })
        ));

        let bytes = encode_pending(&artifact, ArtifactLimits::default()).unwrap();
        let limits = ArtifactLimits {
            max_bytes: bytes.len() - 1,
            ..ArtifactLimits::default()
        };
        assert!(matches!(
            encode_pending(&artifact, limits),
            Err(ArtifactError::Limit {
                field: "pending artifact",
                ..
            })
        ));
        assert!(matches!(
            decode_pending(&bytes, limits),
            Err(ArtifactError::Limit {
                field: "pending artifact",
                ..
            })
        ));
    }

    #[test]
    fn artifact_error_messages_identify_each_failure_class() {
        let errors = [
            ArtifactError::Limit {
                field: "value",
                observed: 2,
                maximum: 1,
            },
            ArtifactError::Allocation {
                field: "value",
                requested: 2,
                detail: "test".to_owned(),
            },
            ArtifactError::Corrupt {
                offset: 1,
                reason: "test",
            },
            ArtifactError::ValueCodec {
                operation: "decode",
                detail: "test".to_owned(),
            },
            ArtifactError::Unsupported { reason: "test" },
            ArtifactError::BindingMismatch,
        ];
        let messages = errors.map(|error| error.to_string());

        assert!(messages.iter().all(|message| !message.is_empty()));
        assert_eq!(
            messages
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            messages.len()
        );
    }
}
