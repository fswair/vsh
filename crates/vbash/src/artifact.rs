use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use vsh_monty::ExecutionStats;
use vsh_policy::{
    AccessKind, DenyReason, RiskFlag, RiskManifest, RiskMetrics, read_set_digest, write_set_digest,
};
use vsh_types::{
    ContentVersion, DiffDigest, DiffEntry, DiffKind, DirectoryDigest, ExecutionEvidenceDigest,
    FileStamp, IntentDigest, NodeKind, NodeState, PlatformFileId, PolicyDigest, ProgramDigest,
    ReadSetDigest, RuntimeConfigDigest, SnapshotId, TransactionBinding, VPath, WriteSetDigest,
};
use vsh_vfs::{
    CanonicalDiff, Effect, EffectEvent, EffectOrigin, ReadObservation, WritePrecondition,
};

use crate::runtime::{ArtifactLimits, Receipt, RuntimeDecision, StageTimings};
use crate::{BashResult, ExecutionOutput};

const ARTIFACT_MAGIC_V1: &[u8; 8] = b"VSHPND01";
const ARTIFACT_MAGIC_V2: &[u8; 8] = b"VSHPND02";
const ARTIFACT_MAGIC_V3: &[u8; 8] = b"VSHPND03";

#[derive(Clone)]
pub(crate) struct ReviewEvidence {
    pub(crate) intent: Option<String>,
    pub(crate) metrics: RiskMetrics,
    pub(crate) effects: Vec<EffectEvent>,
    pub(crate) complete: bool,
    pub(crate) truncated: bool,
}

impl ReviewEvidence {
    pub(crate) fn capture(
        intent: Option<&str>,
        metrics: RiskMetrics,
        effects: Vec<EffectEvent>,
        limits: ArtifactLimits,
    ) -> Result<Self, ArtifactError> {
        encode_review_view(
            ReviewEvidenceView {
                intent,
                metrics,
                effects: &effects,
                complete: true,
                truncated: false,
            },
            limits,
            &mut Encoder::counting(limits.max_bytes),
        )?;
        Ok(Self {
            intent: intent.map(str::to_owned),
            metrics,
            effects,
            complete: true,
            truncated: false,
        })
    }
}

#[derive(Clone, Copy)]
struct ReviewEvidenceView<'a> {
    intent: Option<&'a str>,
    metrics: RiskMetrics,
    effects: &'a [EffectEvent],
    complete: bool,
    truncated: bool,
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

pub(crate) fn execution_evidence_digest(
    result: &ExecutionOutput,
    execution: ExecutionStats,
    review: &ReviewEvidence,
    decision: &RuntimeDecision,
    limits: ArtifactLimits,
) -> Result<ExecutionEvidenceDigest, ArtifactError> {
    let mut output = Encoder::hashing(limits.max_bytes);
    output.push(1)?; // Evidence codec version.
    encode_review_evidence(review, limits, &mut output)?;
    match decision {
        RuntimeDecision::Denied(manifest) => {
            output.push(3)?;
            encode_risk_metrics(manifest.metrics, &mut output)?;
            output.extend_from_slice(manifest.policy.as_bytes())?;
            encode_deny_reason(&manifest.reason, limits, &mut output)?;
        }
        _ => encode_decision(decision, &mut output)?,
    }
    encode_execution_output(result, limits, true, &mut output)?;
    encode_execution_stats(execution, &mut output)?;
    Ok(output.finish_digest())
}

fn validate_execution_evidence(
    artifact: &PendingTransaction,
    limits: ArtifactLimits,
) -> Result<(), ArtifactError> {
    let Some(expected) = artifact.binding.execution_evidence else {
        return Ok(());
    };
    if !artifact.review.complete
        || artifact.review.truncated
        || artifact.binding.intent
            != artifact
                .review
                .intent
                .as_deref()
                .map(IntentDigest::digest_text)
        || expected
            != execution_evidence_digest(
                &artifact.receipt.output,
                artifact.receipt.execution,
                &artifact.review,
                &artifact.receipt.decision,
                limits,
            )?
    {
        return Err(ArtifactError::EvidenceMismatch);
    }
    Ok(())
}

pub(crate) fn encode_pending(
    artifact: &PendingTransaction,
    limits: ArtifactLimits,
) -> Result<Vec<u8>, ArtifactError> {
    let mut output = Encoder::new(limits.max_bytes);
    encode_pending_fields(artifact, limits, &mut output)?;
    Ok(output.finish())
}

#[cfg(test)]
fn pending_encoded_size(
    artifact: &PendingTransaction,
    limits: ArtifactLimits,
) -> Result<usize, ArtifactError> {
    let mut output = Encoder::counting(limits.max_bytes);
    encode_pending_fields(artifact, limits, &mut output)?;
    Ok(output.observed)
}

/// Seal fresh evidence and count the complete artifact in one bounded pass.
/// Never use this to refresh the identity of an existing or loaded transaction.
pub(crate) fn seal_pending_and_size(
    artifact: &mut PendingTransaction,
    limits: ArtifactLimits,
) -> Result<usize, ArtifactError> {
    if artifact.binding.execution_evidence.is_some() {
        return Err(ArtifactError::EvidenceMismatch);
    }
    let mut output = Encoder::counting(limits.max_bytes);
    let digest = write_pending_fields(artifact, limits, &mut output, true)?
        .expect("creation always writes modern evidence");
    artifact.binding.execution_evidence = Some(digest);
    artifact.receipt.transaction = artifact.binding.transaction_id();
    Ok(output.observed)
}

/// Creation-only bounded encoding and sealing, with one evidence traversal.
pub(crate) fn seal_pending_and_encode(
    artifact: &mut PendingTransaction,
    limits: ArtifactLimits,
) -> Result<Vec<u8>, ArtifactError> {
    if artifact.binding.execution_evidence.is_some() {
        return Err(ArtifactError::EvidenceMismatch);
    }
    let mut output = Encoder::new(limits.max_bytes);
    let digest = write_pending_fields(artifact, limits, &mut output, true)?
        .expect("creation always writes modern evidence");
    // The final binding field is the fixed-width evidence digest. Intent's
    // optional digest changes the offset; derive it from the same binding codec.
    let seal_offset = output
        .seal_offset
        .expect("creation reserves the digest slot");
    output.bytes[seal_offset..seal_offset + 32].copy_from_slice(digest.as_bytes());
    artifact.binding.execution_evidence = Some(digest);
    artifact.receipt.transaction = artifact.binding.transaction_id();
    Ok(output.finish())
}

fn encode_pending_fields(
    artifact: &PendingTransaction,
    limits: ArtifactLimits,
    output: &mut Encoder,
) -> Result<(), ArtifactError> {
    write_pending_fields(artifact, limits, output, false).map(|_| ())
}

fn write_pending_fields(
    artifact: &PendingTransaction,
    limits: ArtifactLimits,
    output: &mut Encoder,
    create_seal: bool,
) -> Result<Option<ExecutionEvidenceDigest>, ArtifactError> {
    let modern = create_seal || artifact.binding.execution_evidence.is_some();
    if !modern {
        let new_effects = artifact.review.effects.iter().any(|event| {
            matches!(event.effect, Effect::ModifyMetadata { .. })
                || event.origin == EffectOrigin::BashCall
        });
        let new_risks = matches!(&artifact.receipt.decision, RuntimeDecision::PendingApproval(manifest)
        if manifest.flags.contains(&RiskFlag::PermissionChange));
        if new_effects || new_risks {
            return Err(ArtifactError::Unsupported {
                reason: "metadata effects require the version-three execution artifact",
            });
        }
    }
    output.extend_from_slice(if modern {
        ARTIFACT_MAGIC_V3
    } else {
        ARTIFACT_MAGIC_V2
    })?;
    if create_seal {
        // Fixed-width slot; its contents do not participate in the evidence hash.
        let mut binding = artifact.binding;
        binding.execution_evidence = Some(ExecutionEvidenceDigest::from_bytes([0; 32]));
        encode_binding(&binding, output)?;
        output.seal_offset = Some(output.observed - 32);
    } else {
        encode_binding(&artifact.binding, output)?;
    }
    let digest = encode_receipt_evidence(artifact, limits, output, create_seal)?;

    if artifact.diff.entries().len() > limits.max_entries {
        return Err(ArtifactError::Limit {
            field: "diff entries",
            observed: artifact.diff.entries().len(),
            maximum: limits.max_entries,
        });
    }
    encode_len(artifact.diff.entries().len(), output)?;
    for entry in artifact.diff.entries() {
        encode_path(&entry.path, limits, output)?;
        encode_optional_state(entry.before, output)?;
        encode_optional_state(entry.after, output)?;
        output.push(diff_kind_tag(entry.kind))?;
    }

    if artifact.read_set.len() > limits.max_dependencies {
        return Err(ArtifactError::Limit {
            field: "read dependencies",
            observed: artifact.read_set.len(),
            maximum: limits.max_dependencies,
        });
    }
    encode_len(artifact.read_set.len(), output)?;
    for (path, observation) in &artifact.read_set {
        encode_path(path, limits, output)?;
        match observation.metadata {
            None => output.push(0)?,
            Some(None) => output.push(1)?,
            Some(Some(state)) => {
                output.push(2)?;
                encode_state(state, output)?;
            }
        }
        encode_optional_digest(observation.content.map(|value| *value.as_bytes()), output)?;
        encode_optional_digest(observation.directory.map(|value| *value.as_bytes()), output)?;
    }

    if artifact.write_set.len() > limits.max_dependencies {
        return Err(ArtifactError::Limit {
            field: "write dependencies",
            observed: artifact.write_set.len(),
            maximum: limits.max_dependencies,
        });
    }
    encode_len(artifact.write_set.len(), output)?;
    for (path, precondition) in &artifact.write_set {
        encode_path(path, limits, output)?;
        encode_optional_state(precondition.expected, output)?;
    }

    Ok(digest)
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
    let has_review_evidence = if magic == ARTIFACT_MAGIC_V3 {
        decoder.modern = true;
        true
    } else if magic == ARTIFACT_MAGIC_V2 {
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
    let output = decode_execution_output(&mut decoder, limits)?;
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
        output,
        execution,
        timings,
        commit: None,
    };
    let artifact = PendingTransaction {
        binding,
        diff,
        read_set,
        write_set,
        review,
        receipt,
    };
    validate_execution_evidence(&artifact, limits)?;
    Ok(artifact)
}

fn encode_receipt_evidence(
    artifact: &PendingTransaction,
    limits: ArtifactLimits,
    output: &mut Encoder,
    create_seal: bool,
) -> Result<Option<ExecutionEvidenceDigest>, ArtifactError> {
    let modern = create_seal || artifact.binding.execution_evidence.is_some();
    if modern {
        if !artifact.review.complete
            || artifact.review.truncated
            || artifact.binding.intent
                != artifact
                    .review
                    .intent
                    .as_deref()
                    .map(IntentDigest::digest_text)
        {
            return Err(ArtifactError::EvidenceMismatch);
        }
        let mut digest = HashSink::new();
        digest.extend(&[1]);
        output.hasher = Some(digest);
        output.digest_active = true;
    }
    encode_review_evidence(&artifact.review, limits, output)?;
    output.digest_active = false;
    output.push(u8::from(!artifact.receipt.changes.is_empty()))?;
    output.digest_active = modern;
    encode_decision(&artifact.receipt.decision, output)?;
    encode_execution_output(&artifact.receipt.output, limits, modern, output)?;
    encode_execution_stats(artifact.receipt.execution, output)?;
    output.digest_active = false;
    let digest = if modern {
        let digest = output
            .hasher
            .take()
            .expect("modern receipt initialized its digest")
            .finish();
        if !create_seal && Some(digest) != artifact.binding.execution_evidence {
            return Err(ArtifactError::EvidenceMismatch);
        }
        Some(digest)
    } else {
        None
    };
    encode_timings(artifact.receipt.timings, output)?;
    Ok(digest)
}

fn encode_execution_output(
    result: &ExecutionOutput,
    limits: ArtifactLimits,
    modern: bool,
    output: &mut Encoder,
) -> Result<(), ArtifactError> {
    let (value, stdout) = match result {
        ExecutionOutput::Monty { value, stdout } => (value, stdout),
        ExecutionOutput::Bash(result) => {
            if !modern {
                return Err(ArtifactError::Unsupported {
                    reason: "Bash output requires a version-three artifact",
                });
            }
            if result.exit_code != 0 {
                return Err(ArtifactError::Unsupported {
                    reason: "failed Bash output cannot become a pending transaction",
                });
            }
            check_bash_output_limits(result, limits)?;
            output.extend_from_slice(&[2, 1])?;
            encode_bytes(result.profile.as_bytes(), output)?;
            output.extend_from_slice(&result.exit_code.to_le_bytes())?;
            encode_bytes(&result.stdout, output)?;
            return encode_bytes(&result.stderr, output);
        }
    };
    if modern {
        output.push(1)?; // Monty output, never inferred from source text.
        output.push(1)?; // Complete terminal result.
    }
    let value_len = postcard::experimental::serialized_size(value).map_err(|source| {
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
    encode_len(value_len, output)?;
    let mut failure = None;
    postcard::serialize_with_flavor(
        value,
        ValueFlavor {
            output,
            failure: &mut failure,
        },
    )
    .map_err(|source| {
        failure.unwrap_or_else(|| ArtifactError::ValueCodec {
            operation: "encode",
            detail: source.to_string(),
        })
    })?;
    if stdout.len() > limits.max_stdout_bytes {
        return Err(ArtifactError::Limit {
            field: "stdout",
            observed: stdout.len(),
            maximum: limits.max_stdout_bytes,
        });
    }
    encode_bytes(stdout.as_bytes(), output)
}

fn check_bash_output_limits(
    result: &BashResult,
    limits: ArtifactLimits,
) -> Result<(), ArtifactError> {
    let total = result.stdout.len().saturating_add(result.stderr.len());
    for (field, observed, maximum) in [
        ("Bash profile", result.profile.len(), 128),
        ("result value", 4, limits.max_value_bytes),
        ("stdout/stderr", total, limits.max_stdout_bytes),
    ] {
        if observed > maximum {
            return Err(ArtifactError::Limit {
                field,
                observed,
                maximum,
            });
        }
    }
    Ok(())
}

fn decode_execution_output(
    decoder: &mut Decoder<'_>,
    limits: ArtifactLimits,
) -> Result<ExecutionOutput, ArtifactError> {
    let tag = if decoder.modern {
        let tag = decoder.byte()?;
        if !matches!(tag, 1 | 2) || decoder.byte()? != 1 {
            return Err(decoder.corrupt("unknown or incomplete execution output"));
        }
        tag
    } else {
        1
    };
    if tag == 2 {
        if limits.max_value_bytes < 4 {
            return Err(ArtifactError::Limit {
                field: "Bash status",
                observed: 4,
                maximum: limits.max_value_bytes,
            });
        }
        let profile = decoder.string(128, "Bash profile")?;
        let exit_code = i32::from_le_bytes(decoder.take(4)?.try_into().expect("four status bytes"));
        if exit_code != 0 {
            return Err(decoder.corrupt("failed Bash output in pending artifact"));
        }
        let stdout_bytes = decoder.length_prefixed(limits.max_stdout_bytes, "stdout")?;
        let mut stdout = bounded_vec(stdout_bytes.len(), "stdout")?;
        stdout.extend_from_slice(stdout_bytes);
        let remaining = limits.max_stdout_bytes.saturating_sub(stdout.len());
        let stderr_bytes = decoder.length_prefixed(remaining, "stderr")?;
        let mut stderr = bounded_vec(stderr_bytes.len(), "stderr")?;
        stderr.extend_from_slice(stderr_bytes);
        let result = BashResult {
            profile,
            exit_code,
            stdout,
            stderr,
        };
        check_bash_output_limits(&result, limits)?;
        return Ok(ExecutionOutput::Bash(result));
    }
    let value_bytes = decoder.length_prefixed(limits.max_value_bytes, "result value")?;
    let (value, remainder) =
        postcard::take_from_bytes(value_bytes).map_err(|source| ArtifactError::ValueCodec {
            operation: "decode",
            detail: source.to_string(),
        })?;
    if !remainder.is_empty() {
        return Err(decoder.corrupt("trailing result-value bytes"));
    }
    let stdout = decoder.string(limits.max_stdout_bytes, "stdout")?;
    Ok(ExecutionOutput::Monty { value, stdout })
}

struct ValueFlavor<'a> {
    output: &'a mut Encoder,
    failure: &'a mut Option<ArtifactError>,
}

impl postcard::ser_flavors::Flavor for ValueFlavor<'_> {
    type Output = ();

    fn try_push(&mut self, byte: u8) -> postcard::Result<()> {
        self.try_extend(&[byte])
    }

    fn try_extend(&mut self, bytes: &[u8]) -> postcard::Result<()> {
        self.output.extend_from_slice(bytes).map_err(|error| {
            *self.failure = Some(error);
            postcard::Error::SerializeBufferFull
        })
    }

    fn finalize(self) -> postcard::Result<()> {
        Ok(())
    }
}

fn encode_review_evidence(
    review: &ReviewEvidence,
    limits: ArtifactLimits,
    output: &mut Encoder,
) -> Result<(), ArtifactError> {
    encode_review_view(
        ReviewEvidenceView {
            intent: review.intent.as_deref(),
            metrics: review.metrics,
            effects: &review.effects,
            complete: review.complete,
            truncated: review.truncated,
        },
        limits,
        output,
    )
}

fn encode_review_view(
    review: ReviewEvidenceView<'_>,
    limits: ArtifactLimits,
    output: &mut Encoder,
) -> Result<(), ArtifactError> {
    match review.intent {
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
    for event in review.effects {
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
    let count = decoder.collection_length(limits.max_effects, 19, "review effects")?;
    let mut effects = bounded_vec(count, "review effects")?;
    for _ in 0..count {
        let sequence = decoder.u64()?;
        let origin = decode_effect_origin(decoder.byte()?)
            .ok_or_else(|| decoder.corrupt("unknown effect-origin tag"))?;
        if !decoder.modern && origin == EffectOrigin::BashCall {
            return Err(decoder.corrupt("Bash origin in a legacy Monty artifact"));
        }
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
        Effect::ModifyMetadata {
            path,
            before,
            after,
        } => {
            output.push(8)?;
            encode_path(path, limits, output)?;
            encode_state(*before, output)?;
            encode_state(*after, output)?;
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
            state: decode_optional_state(decoder, true)?,
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
            after: decode_state(decoder, true)?,
        }),
        5 => Ok(Effect::ModifyContent {
            path: decoder.path(limits)?,
            before: decode_state(decoder, true)?,
            after: decode_state(decoder, true)?,
        }),
        6 => Ok(Effect::Delete {
            path: decoder.path(limits)?,
            before: decode_state(decoder, true)?,
        }),
        8 if decoder.modern => Ok(Effect::ModifyMetadata {
            path: decoder.path(limits)?,
            before: decode_state(decoder, true)?,
            after: decode_state(decoder, true)?,
        }),
        7 => Ok(Effect::Rename {
            from: decoder.path(limits)?,
            to: decoder.path(limits)?,
            before: decode_state(decoder, true)?,
            after: decode_state(decoder, true)?,
        }),
        _ => Err(decoder.corrupt("unknown effect tag")),
    }
}

fn effect_origin_tag(origin: EffectOrigin) -> Result<u8, ArtifactError> {
    match origin {
        EffectOrigin::VirtualFs => Ok(1),
        EffectOrigin::MontyOsCall => Ok(2),
        EffectOrigin::MontyToolCall => Ok(3),
        EffectOrigin::BashCall => Ok(4),
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
        4 => Some(EffectOrigin::BashCall),
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
    encode_optional_digest(binding.intent.map(|value| *value.as_bytes()), output)?;
    if let Some(evidence) = binding.execution_evidence {
        output.extend_from_slice(evidence.as_bytes())?;
    }
    Ok(())
}

fn decode_diff(
    decoder: &mut Decoder<'_>,
    limits: ArtifactLimits,
) -> Result<CanonicalDiff, ArtifactError> {
    let entry_count = decoder.collection_length(limits.max_entries, 11, "diff entries")?;
    let mut entries = bounded_vec(entry_count, "diff entries")?;
    for _ in 0..entry_count {
        let path = decoder.path(limits)?;
        let before = decode_optional_state(decoder, false)?;
        let after = decode_optional_state(decoder, true)?;
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
    let count = decoder.collection_length(limits.max_dependencies, 11, "read dependencies")?;
    let mut read_set = BTreeMap::new();
    for _ in 0..count {
        let path = decoder.path(limits)?;
        let metadata = match decoder.byte()? {
            0 => None,
            1 => Some(None),
            2 => Some(Some(decode_state(decoder, false)?)),
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
    let count = decoder.collection_length(limits.max_dependencies, 9, "write dependencies")?;
    let mut write_set = BTreeMap::new();
    for _ in 0..count {
        let path = decoder.path(limits)?;
        let expected = decode_optional_state(decoder, false)?;
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
        execution_evidence: if decoder.modern {
            Some(ExecutionEvidenceDigest::from_bytes(decoder.digest()?))
        } else {
            None
        },
    })
}

fn encode_deny_reason(
    reason: &DenyReason,
    limits: ArtifactLimits,
    output: &mut Encoder,
) -> Result<(), ArtifactError> {
    match reason {
        DenyReason::ProtectedAccessAttempt(access) | DenyReason::ProtectedMutation(access) => {
            output.push(if matches!(reason, DenyReason::ProtectedAccessAttempt(_)) {
                1
            } else {
                2
            })?;
            encode_path(&access.path, limits, output)?;
            output.push(match access.access {
                AccessKind::MetadataRead => 1,
                AccessKind::ContentRead => 2,
                AccessKind::DirectoryRead => 3,
                AccessKind::Create => 4,
                AccessKind::Modify => 5,
                AccessKind::Delete => 6,
                AccessKind::RenameSource => 7,
                AccessKind::RenameDestination => 8,
            })?;
            encode_bytes(access.rule.as_bytes(), output)?;
        }
        DenyReason::TouchedPathLimit { limit, observed }
        | DenyReason::DeletePathLimit { limit, observed } => {
            output.push(if matches!(reason, DenyReason::TouchedPathLimit { .. }) {
                3
            } else {
                5
            })?;
            encode_len(*limit, output)?;
            encode_len(*observed, output)?;
        }
        DenyReason::ChangedByteLimit { limit, observed } => {
            output.push(4)?;
            output.extend_from_slice(&limit.to_le_bytes())?;
            output.extend_from_slice(&observed.to_le_bytes())?;
        }
        DenyReason::DeleteRatioLimit {
            limit_bps,
            observed_bps,
        } => {
            output.push(6)?;
            output.extend_from_slice(&limit_bps.to_le_bytes())?;
            output.extend_from_slice(&observed_bps.to_le_bytes())?;
        }
        _ => {
            return Err(ArtifactError::Unsupported {
                reason: "unknown denial reason",
            });
        }
    }
    Ok(())
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
    let count = decoder.collection_length(32, 1, "risk flags")?;
    let mut flags = bounded_vec(count, "risk flags")?;
    for _ in 0..count {
        let flag = decode_risk_flag(decoder.byte()?)
            .ok_or_else(|| decoder.corrupt("unknown risk-flag tag"))?;
        if !decoder.modern && flag == RiskFlag::PermissionChange {
            return Err(decoder.corrupt("permission risk in a legacy artifact"));
        }
        flags.push(flag);
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

fn decode_optional_state(
    decoder: &mut Decoder<'_>,
    virtual_state: bool,
) -> Result<Option<NodeState>, ArtifactError> {
    match decoder.byte()? {
        0 => Ok(None),
        1 => decode_state(decoder, virtual_state).map(Some),
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

fn decode_state(
    decoder: &mut Decoder<'_>,
    virtual_state: bool,
) -> Result<NodeState, ArtifactError> {
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
            let virtual_mode = decoder.modern
                && virtual_state
                && matches!(kind, NodeKind::File | NodeKind::Directory)
                && mode & !0o777 == 0;
            if stamp.kind != kind || stamp.size != size || (stamp.mode != mode && !virtual_mode) {
                return Err(decoder.corrupt("node state and metadata stamp disagree"));
            }
            NodeState::from_stamp(stamp).with_mode(mode)
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
        RiskFlag::PermissionChange => 8,
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
        8 => Some(RiskFlag::PermissionChange),
        _ => None,
    }
}

fn bounded_vec<T>(count: usize, field: &'static str) -> Result<Vec<T>, ArtifactError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|source| ArtifactError::Allocation {
            field,
            requested: count.saturating_mul(size_of::<T>()),
            detail: source.to_string(),
        })?;
    Ok(values)
}

struct HashSink {
    hasher: blake3::Hasher,
    buffer: [u8; 4_096],
    buffered: usize,
}

impl HashSink {
    fn new() -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"vsh\0execution-evidence-v1\0");
        Self {
            hasher,
            buffer: [0; 4_096],
            buffered: 0,
        }
    }

    fn extend(&mut self, mut bytes: &[u8]) {
        if self.buffered != 0 {
            let count = bytes.len().min(self.buffer.len() - self.buffered);
            self.buffer[self.buffered..self.buffered + count].copy_from_slice(&bytes[..count]);
            self.buffered += count;
            bytes = &bytes[count..];
            if self.buffered == self.buffer.len() {
                self.hasher.update(&self.buffer);
                self.buffered = 0;
            }
        }
        if bytes.len() >= self.buffer.len() {
            self.hasher.update(bytes);
        } else if !bytes.is_empty() {
            self.buffer[..bytes.len()].copy_from_slice(bytes);
            self.buffered = bytes.len();
        }
    }

    fn finish(mut self) -> ExecutionEvidenceDigest {
        self.hasher.update(&self.buffer[..self.buffered]);
        ExecutionEvidenceDigest::from_bytes(*self.hasher.finalize().as_bytes())
    }
}

struct Encoder {
    bytes: Vec<u8>,
    hasher: Option<HashSink>,
    retain_bytes: bool,
    digest_active: bool,
    observed: usize,
    maximum: usize,
    seal_offset: Option<usize>,
}

impl Encoder {
    const fn new(maximum: usize) -> Self {
        Self {
            bytes: Vec::new(),
            hasher: None,
            retain_bytes: true,
            digest_active: false,
            observed: 0,
            maximum,
            seal_offset: None,
        }
    }

    fn hashing(maximum: usize) -> Self {
        Self {
            bytes: Vec::new(),
            hasher: Some(HashSink::new()),
            retain_bytes: false,
            digest_active: true,
            observed: 0,
            maximum,
            seal_offset: None,
        }
    }

    const fn counting(maximum: usize) -> Self {
        Self {
            bytes: Vec::new(),
            hasher: None,
            retain_bytes: false,
            digest_active: false,
            observed: 0,
            maximum,
            seal_offset: None,
        }
    }

    fn push(&mut self, byte: u8) -> Result<(), ArtifactError> {
        self.extend_from_slice(&[byte])
    }

    fn extend_from_slice(&mut self, bytes: &[u8]) -> Result<(), ArtifactError> {
        let required = self
            .observed
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
        if self.digest_active
            && let Some(hasher) = &mut self.hasher
        {
            hasher.extend(bytes);
        }
        if !self.retain_bytes {
            self.observed = required;
            return Ok(());
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
        self.observed = required;
        Ok(())
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }

    fn finish_digest(self) -> ExecutionEvidenceDigest {
        let hasher = self.hasher.expect("digest encoder always has a hash sink");
        hasher.finish()
    }
}

struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
    modern: bool,
}

impl<'a> Decoder<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            offset: 0,
            modern: false,
        }
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

    fn collection_length(
        &mut self,
        maximum: usize,
        minimum_entry_bytes: usize,
        field: &'static str,
    ) -> Result<usize, ArtifactError> {
        let count = self.length(maximum, field)?;
        if count > self.bytes.len().saturating_sub(self.offset) / minimum_entry_bytes {
            return Err(self.corrupt("collection length exceeds remaining artifact bytes"));
        }
        Ok(count)
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
    /// Result, output or review evidence differs from its pre-approval seal.
    EvidenceMismatch,
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
            Self::EvidenceMismatch => formatter.write_str(
                "pending execution result or review evidence does not match its transaction seal",
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
            execution_evidence: None,
        };
        let receipt = Receipt {
            transaction: binding.transaction_id(),
            base_snapshot: binding.base_snapshot,
            state: TransactionState::AutoApproved,
            decision: RuntimeDecision::AutoApproved,
            diff: diff.digest(),
            changed_paths: 1,
            changes: diff.entries().to_vec(),
            output: ExecutionOutput::Monty {
                value: MontyObject::Int(42),
                stdout: "ok\n".to_owned(),
            },
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

    fn seal(artifact: &mut PendingTransaction) {
        artifact.binding.execution_evidence = Some(
            execution_evidence_digest(
                &artifact.receipt.output,
                artifact.receipt.execution,
                &artifact.review,
                &artifact.receipt.decision,
                ArtifactLimits::default(),
            )
            .unwrap(),
        );
        artifact.receipt.transaction = artifact.binding.transaction_id();
    }

    #[test]
    fn fresh_durable_encoding_matches_verified_codec_with_and_without_intent() {
        for intent in [true, false] {
            let mut fresh = fixture();
            if !intent {
                fresh.binding.intent = None;
                fresh.review.intent = None;
            }
            let mut verified = fresh.clone();
            seal(&mut verified);
            let expected = encode_pending(&verified, ArtifactLimits::default()).unwrap();
            let actual = seal_pending_and_encode(&mut fresh, ArtifactLimits::default()).unwrap();
            assert_eq!(actual, expected);
            assert_eq!(fresh.binding, verified.binding);
            assert_eq!(
                decode_pending(&actual, ArtifactLimits::default())
                    .unwrap()
                    .binding,
                fresh.binding
            );
            assert!(matches!(
                seal_pending_and_encode(&mut fresh, ArtifactLimits::default()),
                Err(ArtifactError::EvidenceMismatch)
            ));
        }
        let mut fresh = fixture();
        let identity = fresh.binding;
        assert!(
            seal_pending_and_encode(
                &mut fresh,
                ArtifactLimits {
                    max_bytes: 32,
                    ..ArtifactLimits::default()
                }
            )
            .is_err()
        );
        assert_eq!(fresh.binding, identity);
    }

    #[test]
    fn bash_evidence_is_binary_bound_and_combined_output_limits_apply() {
        let limits = ArtifactLimits::default();
        let mut fresh = fixture();
        fresh.receipt.output = ExecutionOutput::Bash(BashResult {
            profile: "vsh-bash-bounded-v2".into(),
            exit_code: 0,
            stdout: vec![0xff, 0, 0xfe],
            stderr: vec![0x80, 0x81],
        });
        let bytes = seal_pending_and_encode(&mut fresh, limits).unwrap();
        let decoded = decode_pending(&bytes, limits).unwrap();
        assert_eq!(decoded.receipt.output, fresh.receipt.output);
        for stream in [[0xff, 0, 0xfe].as_slice(), [0x80, 0x81].as_slice()] {
            let mut corrupted = bytes.clone();
            let offset = corrupted
                .windows(stream.len())
                .position(|value| value == stream)
                .unwrap();
            corrupted[offset] ^= 1;
            assert!(matches!(
                decode_pending(&corrupted, limits),
                Err(ArtifactError::EvidenceMismatch)
            ));
        }
        assert!(
            decode_pending(
                &bytes,
                ArtifactLimits {
                    max_stdout_bytes: 4,
                    ..limits
                }
            )
            .is_err()
        );
        assert!(
            decode_pending(
                &bytes,
                ArtifactLimits {
                    max_value_bytes: 3,
                    ..limits
                }
            )
            .is_err()
        );
        if let ExecutionOutput::Bash(result) = &mut fresh.receipt.output {
            result.exit_code = 1;
        }
        assert!(encode_pending(&fresh, limits).is_err());
        let mut output = Encoder::new(limits.max_bytes);
        assert!(
            encode_execution_output(&decoded.receipt.output, limits, false, &mut output).is_err()
        );
    }

    #[test]
    fn sealed_artifact_round_trip_binds_result_output_and_review() {
        let mut artifact = fixture();
        let legacy = artifact.binding.transaction_id();
        seal(&mut artifact);
        assert_ne!(artifact.receipt.transaction, legacy);
        let bytes = encode_pending(&artifact, ArtifactLimits::default()).unwrap();
        assert_eq!(&bytes[..8], ARTIFACT_MAGIC_V3);
        let decoded = decode_pending(&bytes, ArtifactLimits::default()).unwrap();
        assert_eq!(decoded.binding, artifact.binding);
        assert_eq!(decoded.receipt.transaction, artifact.receipt.transaction);
        assert_eq!(decoded.receipt.output, artifact.receipt.output);
        assert_eq!(decoded.review.effects, artifact.review.effects);
        assert!(decoded.review.complete);
    }

    #[test]
    fn execution_seal_changes_with_authoritative_evidence_but_not_display() {
        let mut artifact = fixture();
        seal(&mut artifact);
        let changes: [fn(&mut PendingTransaction); 8] = [
            |value| {
                if let ExecutionOutput::Monty { value, .. } = &mut value.receipt.output {
                    *value = MontyObject::Int(43);
                }
            },
            |value| {
                if let ExecutionOutput::Monty { stdout, .. } = &mut value.receipt.output {
                    stdout.push('!');
                }
            },
            |value| value.receipt.execution.os_calls += 1,
            |value| value.review.intent = Some("misleading intent".to_owned()),
            |value| value.review.metrics.changed_bytes += 1,
            |value| value.review.effects[0].sequence += 1,
            |value| value.review.complete = false,
            |value| value.review.truncated = true,
        ];
        for change in changes {
            let mut tampered = artifact.clone();
            change(&mut tampered);
            assert!(matches!(
                encode_pending(&tampered, ArtifactLimits::default()),
                Err(ArtifactError::EvidenceMismatch)
            ));
            seal(&mut tampered);
            assert_ne!(tampered.receipt.transaction, artifact.receipt.transaction);
        }
        let mut display = artifact.clone();
        display.receipt.changes.clear();
        display.receipt.timings.total_ns += 1;
        let bytes = encode_pending(&display, ArtifactLimits::default()).unwrap();
        let decoded = decode_pending(&bytes, ArtifactLimits::default()).unwrap();
        assert_eq!(decoded.binding, artifact.binding);
        assert!(decoded.receipt.changes.is_empty());
    }

    #[test]
    fn sealed_decode_rejects_changed_stdout_and_unknown_or_incomplete_output() {
        let mut artifact = fixture();
        seal(&mut artifact);
        let limits = ArtifactLimits::default();
        let original = encode_pending(&artifact, limits).unwrap();
        let mut bytes = original.clone();
        let stdout = bytes
            .windows(3)
            .position(|window| window == b"ok\n")
            .unwrap();
        bytes[stdout] = b'n';
        assert!(matches!(
            decode_pending(&bytes, limits),
            Err(ArtifactError::EvidenceMismatch)
        ));
        let mut decoder = Decoder::new(&original);
        decoder.take(8).unwrap();
        decoder.modern = true;
        decode_binding(&mut decoder).unwrap();
        decode_review_evidence(&mut decoder, limits).unwrap();
        decoder.byte().unwrap();
        decode_decision(&mut decoder).unwrap();
        let output_offset = decoder.offset;
        for (offset, tag) in [(output_offset, 3), (output_offset + 1, 0)] {
            let mut bytes = original.clone();
            bytes[offset] = tag;
            assert!(matches!(
                decode_pending(&bytes, limits),
                Err(ArtifactError::Corrupt {
                    reason: "unknown or incomplete execution output",
                    ..
                })
            ));
        }
    }

    #[test]
    fn sealed_decode_checks_intent_metrics_effects_and_completeness() {
        let mut artifact = fixture();
        seal(&mut artifact);
        let limits = ArtifactLimits::default();
        let original = encode_pending(&artifact, limits).unwrap();
        let mut decoder = Decoder::new(&original);
        decoder.take(8).unwrap();
        decoder.modern = true;
        decode_binding(&mut decoder).unwrap();
        assert_eq!(decoder.byte().unwrap(), 1);
        let intent_len = decoder.length(limits.max_intent_bytes, "intent").unwrap();
        let intent_offset = decoder.offset;
        decoder.take(intent_len).unwrap();
        let metrics_offset = decoder.offset;
        decode_risk_metrics(&mut decoder).unwrap();
        assert_eq!(decoder.length(limits.max_effects, "effects").unwrap(), 1);
        let sequence_offset = decoder.offset;
        decoder.u64().unwrap();
        let origin_offset = decoder.offset;
        decoder.byte().unwrap();
        decode_effect(&mut decoder, limits).unwrap();
        let complete_offset = decoder.offset;
        for (offset, replacement) in [
            (intent_offset, b'x'),
            (metrics_offset, 2),
            (sequence_offset, 2),
            (origin_offset, 3),
            (complete_offset, 0),
            (complete_offset + 1, 1),
        ] {
            let mut bytes = original.clone();
            bytes[offset] = replacement;
            assert!(
                matches!(
                    decode_pending(&bytes, limits),
                    Err(ArtifactError::EvidenceMismatch)
                ),
                "tamper at {offset} was not rejected by the evidence seal"
            );
        }
    }

    #[test]
    fn result_encoding_streams_the_same_postcard_bytes_into_bounded_hash_and_storage() {
        let limits = ArtifactLimits::default();
        let value = MontyObject::List(vec![
            MontyObject::Bytes(vec![0xff; 256 * 1_024]),
            MontyObject::String("value".to_owned()),
            MontyObject::Float(f64::from_bits(0x7ff8_0000_0000_0001)),
        ]);
        let mut storage = Encoder::new(limits.max_bytes);
        let output = ExecutionOutput::Monty {
            value,
            stdout: "stdout".to_owned(),
        };
        encode_execution_output(&output, limits, true, &mut storage).unwrap();
        let bytes = storage.finish();
        let mut decoder = Decoder::new(&bytes);
        assert_eq!(decoder.byte().unwrap(), 1);
        assert_eq!(decoder.byte().unwrap(), 1);
        let encoded_value = decoder
            .length_prefixed(limits.max_value_bytes, "value")
            .unwrap();
        assert_eq!(
            encoded_value,
            postcard::to_allocvec(output.monty_value().unwrap()).unwrap()
        );
        let mut digest = Encoder::hashing(limits.max_bytes);
        encode_execution_output(&output, limits, true, &mut digest).unwrap();
        assert!(digest.bytes.is_empty());
        assert_eq!(digest.bytes.capacity(), 0);
        assert_eq!(digest.observed, bytes.len());
        let mut expected = blake3::Hasher::new();
        expected.update(b"vsh\0execution-evidence-v1\0");
        expected.update(&bytes);
        assert_eq!(
            digest.finish_digest().as_bytes(),
            expected.finalize().as_bytes()
        );
        let mut small = Encoder::new(32);
        assert!(matches!(
            encode_execution_output(&output, limits, true, &mut small),
            Err(ArtifactError::Limit {
                field: "pending artifact",
                ..
            })
        ));
        assert!(small.bytes.len() <= 32);
    }

    #[test]
    fn decoded_collection_counts_are_bounded_by_remaining_bytes_before_allocation() {
        let count = 250_000_u64.to_le_bytes();
        for minimum in [1, 9, 11, 19] {
            assert!(matches!(
                Decoder::new(&count).collection_length(250_000, minimum, "test"),
                Err(ArtifactError::Corrupt {
                    reason: "collection length exceeds remaining artifact bytes",
                    ..
                })
            ));
        }
        let limits = ArtifactLimits {
            max_intent_bytes: 0,
            ..ArtifactLimits::default()
        };
        assert!(matches!(
            ReviewEvidence::capture(Some("intent"), RiskMetrics::default(), vec![], limits),
            Err(ArtifactError::Limit {
                field: "intent",
                ..
            })
        ));
        let mut artifact = fixture();
        let limits = ArtifactLimits {
            max_effects: 0,
            ..ArtifactLimits::default()
        };
        assert!(matches!(
            ReviewEvidence::capture(
                None,
                RiskMetrics::default(),
                artifact.review.effects.clone(),
                limits
            ),
            Err(ArtifactError::Limit {
                field: "review effects",
                ..
            })
        ));
        artifact.binding.execution_evidence = None;
        let bytes = encode_pending(&artifact, ArtifactLimits::default()).unwrap();
        assert_eq!(&bytes[..8], ARTIFACT_MAGIC_V2);
    }

    #[test]
    fn review_capture_checks_path_and_aggregate_bytes_before_retaining_effects() {
        let artifact = fixture();
        for (limits, field) in [
            (
                ArtifactLimits {
                    max_path_bytes: 3,
                    ..ArtifactLimits::default()
                },
                "path",
            ),
            (
                ArtifactLimits {
                    max_bytes: 16,
                    ..ArtifactLimits::default()
                },
                "pending artifact",
            ),
        ] {
            assert!(
                matches!(ReviewEvidence::capture(None, artifact.review.metrics, artifact.review.effects.clone(), limits),
                Err(ArtifactError::Limit { field: actual, .. }) if actual == field)
            );
        }
        let mut counting = Encoder::counting(ArtifactLimits::default().max_bytes);
        encode_review_evidence(&artifact.review, ArtifactLimits::default(), &mut counting).unwrap();
        assert!(counting.observed > 0);
        assert_eq!(counting.bytes.capacity(), 0);
        assert!(counting.hasher.is_none());
    }

    #[test]
    fn version_three_mode_evidence_preserves_lazy_identity_without_relaxing_preconditions() {
        let mut artifact = fixture();
        let before = NodeState::from_stamp(FileStamp {
            kind: NodeKind::File,
            size: 6,
            mode: 0o644,
            mtime_ns: 1,
            ctime_ns: Some(2),
            file_id: PlatformFileId { high: 3, low: 4 },
        });
        let after = before.with_mode(0o600);
        let path = VPath::parse("result.txt").unwrap();
        artifact.diff = CanonicalDiff::from_entries(vec![DiffEntry {
            path: path.clone(),
            before: Some(before),
            after: Some(after),
            kind: DiffKind::MetadataChange,
        }])
        .unwrap();
        artifact.write_set = BTreeMap::from([(
            path.clone(),
            WritePrecondition {
                expected: Some(before),
            },
        )]);
        artifact.binding.diff = artifact.diff.digest();
        artifact.binding.write_set = write_set_digest(&artifact.write_set);
        artifact.review.metrics = RiskMetrics {
            touched_paths: 1,
            modified_paths: 1,
            ..RiskMetrics::default()
        };
        artifact.review.effects = vec![EffectEvent {
            sequence: 1,
            origin: EffectOrigin::BashCall,
            effect: Effect::ModifyMetadata {
                path,
                before,
                after,
            },
        }];
        artifact.receipt.decision = RuntimeDecision::PendingApproval(RiskManifest {
            metrics: artifact.review.metrics,
            flags: vec![RiskFlag::PermissionChange],
            policy: artifact.binding.policy,
        });
        artifact.receipt.diff = artifact.diff.digest();
        artifact.receipt.changes = artifact.diff.entries().to_vec();
        seal(&mut artifact);
        let bytes = encode_pending(&artifact, ArtifactLimits::default()).unwrap();
        let decoded = decode_pending(&bytes, ArtifactLimits::default()).unwrap();
        assert_eq!(decoded.diff, artifact.diff);
        assert_eq!(decoded.review.effects, artifact.review.effects);
        assert_eq!(decoded.write_set, artifact.write_set);

        let mut encoded = Encoder::new(1_024);
        encode_state(after, &mut encoded).unwrap();
        let bytes = encoded.finish();
        let mut strict = Decoder::new(&bytes);
        strict.modern = true;
        assert!(decode_state(&mut strict, false).is_err());
        let mut virtual_state = Decoder::new(&bytes);
        virtual_state.modern = true;
        assert_eq!(decode_state(&mut virtual_state, true).unwrap(), after);
        assert!(decode_state(&mut Decoder::new(&bytes), true).is_err());
        artifact.binding.execution_evidence = None;
        assert!(matches!(
            encode_pending(&artifact, ArtifactLimits::default()),
            Err(ArtifactError::Unsupported { .. })
        ));
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
        assert_eq!(
            decoded.receipt.output,
            ExecutionOutput::Monty {
                value: MontyObject::Int(42),
                stdout: "ok\n".to_owned()
            }
        );
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
    fn each_artifact_version_rejects_trailing_bytes_inside_its_result_field() {
        let limits = ArtifactLimits::default();
        for version in [1, 2, 3] {
            let mut artifact = fixture();
            if version == 3 {
                seal(&mut artifact);
            }
            let mut bytes = encode_pending(&artifact, limits).unwrap();
            if version == 1 {
                let mut decoder = Decoder::new(&bytes);
                decoder.take(8).unwrap();
                decode_binding(&mut decoder).unwrap();
                let review_start = decoder.offset;
                decode_review_evidence(&mut decoder, limits).unwrap();
                let review_end = decoder.offset;
                bytes.drain(review_start..review_end);
                bytes[..8].copy_from_slice(ARTIFACT_MAGIC_V1);
            }
            assert!(decode_pending(&bytes, limits).is_ok(), "version {version}");
            let mut decoder = Decoder::new(&bytes);
            decoder.modern = version == 3;
            decoder.take(8).unwrap();
            decode_binding(&mut decoder).unwrap();
            if version != 1 {
                decode_review_evidence(&mut decoder, limits).unwrap();
            }
            decoder.byte().unwrap();
            decode_decision(&mut decoder).unwrap();
            if version == 3 {
                decoder.take(2).unwrap();
            }
            let length_offset = decoder.offset;
            let value_length = decoder
                .length_prefixed(limits.max_value_bytes, "value")
                .unwrap()
                .len();
            let value_end = decoder.offset;
            let malformed_length = u64::try_from(value_length + 1).unwrap().to_le_bytes();
            bytes[length_offset..length_offset + 8].copy_from_slice(&malformed_length);
            bytes.insert(value_end, 0xff);
            assert!(
                matches!(
                    decode_pending(&bytes, limits),
                    Err(ArtifactError::Corrupt {
                        reason: "trailing result-value bytes",
                        ..
                    })
                ),
                "version {version} accepted a malformed length-delimited result"
            );
        }
    }

    #[test]
    fn counted_pending_size_matches_storage_without_retaining_an_encoded_buffer() {
        for modern in [false, true] {
            let mut artifact = fixture();
            artifact.receipt.output = ExecutionOutput::Monty {
                value: MontyObject::Bytes(vec![0xff; 64 * 1_024]),
                stdout: "result: ✓\n".to_owned(),
            };
            artifact.receipt.execution.output_bytes = artifact.receipt.output.stdout_bytes().len();
            if modern {
                seal(&mut artifact);
            }
            let limits = ArtifactLimits::default();
            let encoded = encode_pending(&artifact, limits).unwrap();
            let mut counting = Encoder::counting(limits.max_bytes);
            encode_pending_fields(&artifact, limits, &mut counting).unwrap();
            assert_eq!(counting.observed, encoded.len());
            assert!(counting.bytes.is_empty());
            assert_eq!(counting.bytes.capacity(), 0);
            assert_eq!(
                pending_encoded_size(&artifact, limits).unwrap(),
                encoded.len()
            );
            let exact = ArtifactLimits {
                max_bytes: encoded.len(),
                ..limits
            };
            assert_eq!(
                pending_encoded_size(&artifact, exact).unwrap(),
                encoded.len()
            );
            let small = ArtifactLimits {
                max_bytes: encoded.len() - 1,
                ..limits
            };
            assert!(matches!(
                pending_encoded_size(&artifact, small),
                Err(ArtifactError::Limit {
                    field: "pending artifact",
                    ..
                })
            ));
            assert!(matches!(
                encode_pending(&artifact, small),
                Err(ArtifactError::Limit {
                    field: "pending artifact",
                    ..
                })
            ));
        }
    }

    #[test]
    fn fresh_seal_matches_verified_encoding_and_is_never_refreshed() {
        let limits = ArtifactLimits::default();
        let mut expected = fixture();
        expected.receipt.output = ExecutionOutput::Monty {
            value: MontyObject::Bytes(vec![0xff; 64 * 1_024]),
            stdout: "result: ✓\n".to_owned(),
        };
        let mut fresh = expected.clone();
        seal(&mut expected);
        let bytes = encode_pending(&expected, limits).unwrap();
        let exact = ArtifactLimits {
            max_bytes: bytes.len(),
            ..limits
        };
        assert_eq!(
            seal_pending_and_size(&mut fresh, exact).unwrap(),
            bytes.len()
        );
        assert_eq!(fresh.binding, expected.binding);
        assert_eq!(fresh.receipt.transaction, expected.receipt.transaction);
        assert_eq!(encode_pending(&fresh, exact).unwrap(), bytes);
        assert_eq!(
            decode_pending(&bytes, exact).unwrap().binding,
            fresh.binding
        );
        assert!(matches!(
            seal_pending_and_size(&mut fresh, exact),
            Err(ArtifactError::EvidenceMismatch)
        ));
        if let ExecutionOutput::Monty { stdout, .. } = &mut fresh.receipt.output {
            stdout.push('x');
        }
        assert!(matches!(
            encode_pending(&fresh, limits),
            Err(ArtifactError::EvidenceMismatch)
        ));
    }

    #[test]
    fn failed_fresh_seal_leaves_identity_unchanged() {
        let limits = ArtifactLimits::default();
        let mut expected = fixture();
        seal(&mut expected);
        let size = encode_pending(&expected, limits).unwrap().len();
        for bounded in [
            ArtifactLimits {
                max_bytes: size - 1,
                ..limits
            },
            ArtifactLimits {
                max_value_bytes: 0,
                ..limits
            },
            ArtifactLimits {
                max_entries: 0,
                ..limits
            },
        ] {
            let mut fresh = fixture();
            let binding = fresh.binding;
            let transaction = fresh.receipt.transaction;
            assert!(matches!(
                seal_pending_and_size(&mut fresh, bounded),
                Err(ArtifactError::Limit { .. })
            ));
            assert_eq!(fresh.binding, binding);
            assert_eq!(fresh.receipt.transaction, transaction);
        }
        let mut incomplete = fixture();
        incomplete.review.complete = false;
        assert!(matches!(
            seal_pending_and_size(&mut incomplete, limits),
            Err(ArtifactError::EvidenceMismatch)
        ));
        assert!(incomplete.binding.execution_evidence.is_none());
    }

    #[test]
    fn counting_retains_the_same_value_limits_and_evidence_verification() {
        let mut artifact = fixture();
        seal(&mut artifact);
        let small = ArtifactLimits {
            max_value_bytes: 0,
            ..ArtifactLimits::default()
        };
        assert!(matches!(
            pending_encoded_size(&artifact, small),
            Err(ArtifactError::Limit {
                field: "result value",
                ..
            })
        ));
        assert!(matches!(
            encode_pending(&artifact, small),
            Err(ArtifactError::Limit {
                field: "result value",
                ..
            })
        ));
        if let ExecutionOutput::Monty { stdout, .. } = &mut artifact.receipt.output {
            stdout.push('x');
        }
        assert!(matches!(
            pending_encoded_size(&artifact, ArtifactLimits::default()),
            Err(ArtifactError::EvidenceMismatch)
        ));
        assert!(matches!(
            encode_pending(&artifact, ArtifactLimits::default()),
            Err(ArtifactError::EvidenceMismatch)
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
            ArtifactError::EvidenceMismatch,
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
