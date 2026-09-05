//! Deterministic capability and transaction policy for VSH.
//!
//! The call policy runs before bytes or mutations reach [`vsh_vfs::VirtualFs`]. The
//! transaction policy then evaluates the observed diff and any denied attempts. Both
//! paths are pure, synchronous, and use only canonical VSH value types.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::ops::BitOr;

use vsh_types::{
    DiffKind, IntentDigest, NodeKind, NodeState, PolicyDigest, ProgramDigest, ReadSetDigest,
    RuntimeConfigDigest, SnapshotId, TransactionBinding, VPath, WriteSetDigest,
};
use vsh_vfs::{CanonicalDiff, Effect, EffectEvent, ReadObservation, WritePrecondition};

/// Version of the canonical deterministic-policy encoding.
pub const POLICY_SCHEMA_VERSION: &str = "vsh-policy-v1";

/// Secret-like paths denied by the default call policy for every access kind.
pub const DEFAULT_SECRET_PATTERNS: &[&str] = &[
    ".env",
    ".env/**",
    ".env.*",
    ".env.*/**",
    "**/.env",
    "**/.env/**",
    "**/.env.*",
    "**/.env.*/**",
    "**/secrets/**",
    "**/id_rsa",
    "**/id_rsa.pub",
    "*.pem",
    "*.key",
    "**/*.pem",
    "**/*.key",
    "**/credentials.json",
    "**/*_credentials.json",
    "**/.ssh/**",
];

/// Trusted runtime paths that untrusted code may neither observe nor mutate.
pub const INTERNAL_RUNTIME_PATTERNS: &[&str] = &[
    ".vsh-runtime",
    ".vsh-runtime/**",
    ".vsh-runtime-owner",
    "**/.vsh-runtime-owner",
];

/// Semantic capability requested for one virtual path.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AccessKind {
    /// Existence or node metadata.
    MetadataRead,
    /// File or symbolic-link content.
    ContentRead,
    /// Directory child enumeration.
    DirectoryRead,
    /// Creation of a previously absent path.
    Create,
    /// Content or metadata replacement.
    Modify,
    /// Removal of an existing path.
    Delete,
    /// Removal side of a rename.
    RenameSource,
    /// Creation/replacement side of a rename.
    RenameDestination,
}

impl AccessKind {
    const fn bit(self) -> u16 {
        match self {
            Self::MetadataRead => 1 << 0,
            Self::ContentRead => 1 << 1,
            Self::DirectoryRead => 1 << 2,
            Self::Create => 1 << 3,
            Self::Modify => 1 << 4,
            Self::Delete => 1 << 5,
            Self::RenameSource => 1 << 6,
            Self::RenameDestination => 1 << 7,
        }
    }

    /// Return whether this access may change virtual state.
    #[must_use]
    pub const fn is_mutation(self) -> bool {
        matches!(
            self,
            Self::Create
                | Self::Modify
                | Self::Delete
                | Self::RenameSource
                | Self::RenameDestination
        )
    }
}

/// Compact set of path capabilities denied by a protected rule.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AccessSet(u16);

impl AccessSet {
    /// Empty capability set.
    pub const NONE: Self = Self(0);
    /// Existence and metadata access.
    pub const METADATA_READ: Self = Self(AccessKind::MetadataRead.bit());
    /// File/link byte access.
    pub const CONTENT_READ: Self = Self(AccessKind::ContentRead.bit());
    /// Directory enumeration access.
    pub const DIRECTORY_READ: Self = Self(AccessKind::DirectoryRead.bit());
    /// All read-like access.
    pub const READS: Self = Self(
        AccessKind::MetadataRead.bit()
            | AccessKind::ContentRead.bit()
            | AccessKind::DirectoryRead.bit(),
    );
    /// Path creation.
    pub const CREATE: Self = Self(AccessKind::Create.bit());
    /// Content or metadata modification.
    pub const MODIFY: Self = Self(AccessKind::Modify.bit());
    /// Path deletion.
    pub const DELETE: Self = Self(AccessKind::Delete.bit());
    /// Source side of rename.
    pub const RENAME_SOURCE: Self = Self(AccessKind::RenameSource.bit());
    /// Destination side of rename.
    pub const RENAME_DESTINATION: Self = Self(AccessKind::RenameDestination.bit());
    /// All state-changing access.
    pub const MUTATIONS: Self = Self(
        AccessKind::Create.bit()
            | AccessKind::Modify.bit()
            | AccessKind::Delete.bit()
            | AccessKind::RenameSource.bit()
            | AccessKind::RenameDestination.bit(),
    );
    /// Every currently defined capability.
    pub const ALL: Self = Self(Self::READS.0 | Self::MUTATIONS.0);

    /// Return whether `access` is in this set.
    #[must_use]
    pub const fn contains(self, access: AccessKind) -> bool {
        self.0 & access.bit() != 0
    }

    const fn bits(self) -> u16 {
        self.0
    }
}

impl BitOr for AccessSet {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// Invalid protected-path pattern.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PatternError {
    /// The pattern was empty.
    Empty,
    /// Absolute patterns are not allowed.
    Absolute,
    /// Backslashes would make matching host-dependent.
    Backslash,
    /// A NUL byte was present.
    NulByte,
    /// Parent traversal is forbidden.
    ParentComponent,
    /// `**` is supported only as a complete path component.
    InvalidGlobstar,
    /// The deliberately small policy language does not support this metacharacter.
    UnsupportedMetacharacter,
}

impl fmt::Display for PatternError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "protected path pattern must not be empty",
            Self::Absolute => "protected path pattern must be relative",
            Self::Backslash => "protected path pattern must use portable separators",
            Self::NulByte => "protected path pattern contains a NUL byte",
            Self::ParentComponent => "protected path pattern contains parent traversal",
            Self::InvalidGlobstar => "globstar must occupy a complete path component",
            Self::UnsupportedMetacharacter => {
                "protected path pattern supports only literal text, '*' and complete '**' components"
            }
        })
    }
}

impl Error for PatternError {}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PathPattern {
    source: String,
    basename_only: bool,
    components: Vec<String>,
}

impl PathPattern {
    fn compile(source: impl Into<String>) -> Result<Self, PatternError> {
        let source = source.into();
        if source.is_empty() {
            return Err(PatternError::Empty);
        }
        if source.starts_with('/') {
            return Err(PatternError::Absolute);
        }
        if source.contains('\\') {
            return Err(PatternError::Backslash);
        }
        if source.contains('\0') {
            return Err(PatternError::NulByte);
        }
        if source.contains(['?', '[', ']']) {
            return Err(PatternError::UnsupportedMetacharacter);
        }

        let mut components = Vec::new();
        for component in source.split('/') {
            if matches!(component, "" | ".") {
                continue;
            }
            if component == ".." {
                return Err(PatternError::ParentComponent);
            }
            if component.contains("**") && component != "**" {
                return Err(PatternError::InvalidGlobstar);
            }
            components.push(component.to_owned());
        }
        if components.is_empty() {
            return Err(PatternError::Empty);
        }
        Ok(Self {
            // Any leading globstars can absorb the entire parent path, so
            // **/*.key has exactly the same basename semantics as *.key.
            basename_only: components.last().is_some_and(|part| part != "**")
                && components[..components.len() - 1]
                    .iter()
                    .all(|part| part == "**"),
            source,
            components,
        })
    }

    fn matches(&self, path: &VPath) -> bool {
        if self.basename_only {
            return path.file_name().is_some_and(|name| {
                component_matches(
                    self.components.last().expect("compiled non-empty pattern"),
                    name,
                )
            });
        }
        path_components_match(
            &self.components,
            if path.is_root() { "" } else { path.as_str() },
        )
    }
}

fn component_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let (mut pattern_index, mut value_index) = (0, 0);
    let (mut last_star, mut star_value_index) = (None, 0);

    while value_index < value.len() {
        if pattern_index < pattern.len() && pattern[pattern_index] == value[value_index] {
            pattern_index += 1;
            value_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            last_star = Some(pattern_index);
            pattern_index += 1;
            star_value_index = value_index;
        } else if let Some(star) = last_star {
            star_value_index += 1;
            value_index = star_value_index;
            pattern_index = star + 1;
        } else {
            return false;
        }
    }
    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

fn path_components_match(pattern: &[String], path: &str) -> bool {
    // A globstar consumes zero or more whole components. Only the most recent
    // globstar needs a retry point: an earlier one cannot help once a later one
    // has matched. Cloneable string cursors avoid per-rule heap allocations and
    // recursion, with O(pattern components * path components) component matches.
    let mut remaining = path.split('/').filter(|component| !component.is_empty());
    let mut pattern_index = 0;
    let mut retry = None;
    while let Some(value) = remaining.clone().next() {
        if pattern.get(pattern_index).is_some_and(|part| part == "**") {
            pattern_index += 1;
            if pattern_index == pattern.len() {
                return true;
            }
            retry = Some((pattern_index, remaining.clone()));
        } else if pattern
            .get(pattern_index)
            .is_some_and(|part| component_matches(part, value))
        {
            pattern_index += 1;
            remaining.next();
        } else if let Some((restart, cursor)) = &mut retry {
            cursor.next();
            remaining = cursor.clone();
            pattern_index = *restart;
        } else {
            return false;
        }
    }
    pattern[pattern_index..].iter().all(|part| part == "**")
}

/// One canonical protected-path rule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtectedRule {
    pattern: PathPattern,
    denied: AccessSet,
}

impl ProtectedRule {
    /// Compile a protected rule in VSH's bounded portable pattern language.
    ///
    /// # Errors
    ///
    /// Returns [`PatternError`] for ambiguous or unsupported patterns.
    pub fn new(pattern: impl Into<String>, denied: AccessSet) -> Result<Self, PatternError> {
        Ok(Self {
            pattern: PathPattern::compile(pattern)?,
            denied,
        })
    }

    /// Return the exact canonical pattern.
    #[must_use]
    pub fn pattern(&self) -> &str {
        &self.pattern.source
    }

    /// Return capabilities denied by this rule.
    #[must_use]
    pub const fn denied(&self) -> AccessSet {
        self.denied
    }
}

/// A denied path capability, retained even when sandboxed code catches the exception.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeniedAccess {
    /// Normalized virtual path that was denied.
    pub path: VPath,
    /// Requested semantic capability.
    pub access: AccessKind,
    /// Canonical rule pattern responsible for denial.
    pub rule: String,
}

/// Immutable pre-call policy used on the Monty hot path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallPolicy {
    rules: Vec<ProtectedRule>,
}

impl CallPolicy {
    /// Construct a policy and canonicalize rule order and duplicates.
    #[must_use]
    pub fn new(mut rules: Vec<ProtectedRule>) -> Self {
        rules.sort_by(|left, right| {
            left.pattern()
                .cmp(right.pattern())
                .then_with(|| left.denied.bits().cmp(&right.denied.bits()))
        });
        let mut canonical: Vec<ProtectedRule> = Vec::with_capacity(rules.len());
        for rule in rules {
            if let Some(previous) = canonical.last_mut()
                && previous.pattern() == rule.pattern()
            {
                previous.denied = previous.denied | rule.denied;
            } else {
                canonical.push(rule);
            }
        }
        Self { rules: canonical }
    }

    /// Build the secure default secret policy plus mutation-only `.git` protection.
    ///
    /// # Panics
    ///
    /// Panics only if a compile-time built-in path pattern violates VSH's pattern DSL.
    #[must_use]
    pub fn secure_default() -> Self {
        let mut rules = DEFAULT_SECRET_PATTERNS
            .iter()
            .chain(INTERNAL_RUNTIME_PATTERNS)
            .map(|pattern| {
                ProtectedRule::new(*pattern, AccessSet::ALL)
                    .expect("built-in protected patterns are valid")
            })
            .collect::<Vec<_>>();
        for pattern in [".git", ".git/**"] {
            rules.push(
                ProtectedRule::new(pattern, AccessSet::MUTATIONS)
                    .expect("built-in git patterns are valid"),
            );
        }
        Self::new(rules)
    }

    /// Return the first deterministic denial for `path`, if any.
    ///
    /// # Errors
    ///
    /// Returns the matching [`DeniedAccess`] when the requested capability is protected.
    pub fn authorize(&self, path: &VPath, access: AccessKind) -> Result<(), DeniedAccess> {
        for rule in &self.rules {
            if rule.denied.contains(access) && rule.pattern.matches(path) {
                return Err(DeniedAccess {
                    path: path.clone(),
                    access,
                    rule: rule.pattern.source.clone(),
                });
            }
        }
        Ok(())
    }

    /// Return canonical policy rules.
    #[must_use]
    pub fn rules(&self) -> &[ProtectedRule] {
        &self.rules
    }

    fn encode_canonical(&self, output: &mut Vec<u8>) {
        encode_usize(self.rules.len(), output);
        for rule in &self.rules {
            encode_bytes(rule.pattern().as_bytes(), output);
            output.extend_from_slice(&rule.denied.bits().to_le_bytes());
        }
    }
}

impl Default for CallPolicy {
    fn default() -> Self {
        Self::secure_default()
    }
}

/// Built-in deterministic transaction posture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyProfile {
    /// Small non-destructive edits may proceed without a judge.
    Balanced,
    /// Every mutation escalates; catastrophic changes are denied.
    Strict,
    /// Every mutation escalates with tighter hard-denial ceilings.
    Paranoid,
}

impl PolicyProfile {
    const fn tag(self) -> u8 {
        match self {
            Self::Balanced => 1,
            Self::Strict => 2,
            Self::Paranoid => 3,
        }
    }
}

/// Deterministic thresholds for escalation and hard denial.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyThresholds {
    /// Escalate at or above this number of touched paths.
    pub escalate_touched_paths: usize,
    /// Escalate at or above this many changed bytes.
    pub escalate_changed_bytes: u64,
    /// Hard-deny above this number of touched paths.
    pub deny_touched_paths: usize,
    /// Hard-deny above this many changed bytes.
    pub deny_changed_bytes: u64,
    /// Hard-deny above this number of deleted paths.
    pub deny_deleted_paths: usize,
    /// Minimum delete count before the ratio ceiling applies.
    pub delete_ratio_minimum_paths: usize,
    /// Hard-deny at or above this deletion ratio in basis points.
    pub deny_delete_ratio_bps: u16,
}

impl PolicyThresholds {
    /// Return thresholds for a built-in profile.
    #[must_use]
    pub const fn for_profile(profile: PolicyProfile) -> Self {
        match profile {
            PolicyProfile::Balanced => Self {
                escalate_touched_paths: 500,
                escalate_changed_bytes: 64 * 1024 * 1024,
                deny_touched_paths: 50_000,
                deny_changed_bytes: 1024 * 1024 * 1024,
                deny_deleted_paths: 10_000,
                delete_ratio_minimum_paths: 100,
                deny_delete_ratio_bps: 7_500,
            },
            PolicyProfile::Strict => Self {
                escalate_touched_paths: 100,
                escalate_changed_bytes: 8 * 1024 * 1024,
                deny_touched_paths: 10_000,
                deny_changed_bytes: 256 * 1024 * 1024,
                deny_deleted_paths: 2_000,
                delete_ratio_minimum_paths: 25,
                deny_delete_ratio_bps: 5_000,
            },
            PolicyProfile::Paranoid => Self {
                escalate_touched_paths: 25,
                escalate_changed_bytes: 1024 * 1024,
                deny_touched_paths: 5_000,
                deny_changed_bytes: 128 * 1024 * 1024,
                deny_deleted_paths: 500,
                delete_ratio_minimum_paths: 10,
                deny_delete_ratio_bps: 2_500,
            },
        }
    }
}

/// Invalid deterministic-policy threshold configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PolicyConfigError {
    /// A touched-path threshold was zero or escalation exceeded denial.
    TouchedPathThreshold,
    /// A changed-byte threshold was zero or escalation exceeded denial.
    ChangedByteThreshold,
    /// A delete threshold was zero.
    DeleteThreshold,
    /// Deletion ratio was outside 1..=10,000 basis points.
    DeleteRatio,
}

impl fmt::Display for PolicyConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TouchedPathThreshold => "invalid touched-path policy thresholds",
            Self::ChangedByteThreshold => "invalid changed-byte policy thresholds",
            Self::DeleteThreshold => "invalid delete policy thresholds",
            Self::DeleteRatio => "delete ratio must be within 1..=10000 basis points",
        })
    }
}

impl Error for PolicyConfigError {}

/// Deterministic transaction policy and its pre-call capability rules.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionPolicy {
    profile: PolicyProfile,
    thresholds: PolicyThresholds,
    call_policy: CallPolicy,
    digest: PolicyDigest,
}

impl TransactionPolicy {
    /// Construct and validate an exact policy configuration.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyConfigError`] for contradictory or zero thresholds.
    pub fn new(
        profile: PolicyProfile,
        thresholds: PolicyThresholds,
        call_policy: CallPolicy,
    ) -> Result<Self, PolicyConfigError> {
        validate_thresholds(thresholds)?;
        let digest = policy_digest(profile, thresholds, &call_policy);
        Ok(Self {
            profile,
            thresholds,
            call_policy,
            digest,
        })
    }

    /// Construct a built-in profile with secure default protected paths.
    ///
    /// # Panics
    ///
    /// Panics only if compile-time built-in thresholds become internally contradictory.
    #[must_use]
    pub fn preset(profile: PolicyProfile) -> Self {
        Self::new(
            profile,
            PolicyThresholds::for_profile(profile),
            CallPolicy::default(),
        )
        .expect("built-in policy thresholds are valid")
    }

    /// Return the built-in profile.
    #[must_use]
    pub const fn profile(&self) -> PolicyProfile {
        self.profile
    }

    /// Return exact deterministic thresholds.
    #[must_use]
    pub const fn thresholds(&self) -> PolicyThresholds {
        self.thresholds
    }

    /// Return the pre-call capability policy.
    #[must_use]
    pub const fn call_policy(&self) -> &CallPolicy {
        &self.call_policy
    }

    /// Return the canonical digest bound into transaction identity.
    #[must_use]
    pub const fn digest(&self) -> PolicyDigest {
        self.digest
    }

    /// Evaluate exact observed artifacts without I/O or mutable global state.
    #[must_use]
    pub fn evaluate(&self, input: PolicyInput<'_>) -> PolicyDecision {
        self.evaluate_with_metrics(input).0
    }

    /// Evaluate once and return the exact metrics used for the decision.
    ///
    /// This avoids a second diff/effect traversal when a caller must retain review
    /// evidence for an external commit hook.
    #[must_use]
    pub fn evaluate_with_metrics(&self, input: PolicyInput<'_>) -> (PolicyDecision, RiskMetrics) {
        let metrics = RiskMetrics::from_evidence(input.diff, input.effects, input.base_node_count);
        let decision = self.evaluate_observed(&input, metrics);
        (decision, metrics)
    }

    fn evaluate_observed(&self, input: &PolicyInput<'_>, metrics: RiskMetrics) -> PolicyDecision {
        if let Some(denial) = self.hard_denial(input, metrics) {
            return denial;
        }
        if input.diff.is_empty() {
            return PolicyDecision::AutoApprove;
        }

        let flags = self.risk_flags(metrics);
        if flags.is_empty() {
            PolicyDecision::AutoApprove
        } else {
            PolicyDecision::Escalate(RiskManifest {
                metrics,
                flags: flags.into_iter().collect(),
                policy: self.digest,
            })
        }
    }

    fn hard_denial(&self, input: &PolicyInput<'_>, metrics: RiskMetrics) -> Option<PolicyDecision> {
        if let Some(attempt) = input.denied_accesses.first() {
            return Some(PolicyDecision::Deny(DenyManifest {
                reason: DenyReason::ProtectedAccessAttempt(attempt.clone()),
                metrics,
                policy: self.digest,
            }));
        }

        for entry in input.diff.entries() {
            let access = match entry.kind {
                DiffKind::Create => AccessKind::Create,
                DiffKind::Delete => AccessKind::Delete,
                DiffKind::Modify | DiffKind::MetadataChange => AccessKind::Modify,
            };
            if let Err(denial) = self.call_policy.authorize(&entry.path, access) {
                return Some(PolicyDecision::Deny(DenyManifest {
                    reason: DenyReason::ProtectedMutation(denial),
                    metrics,
                    policy: self.digest,
                }));
            }
        }

        if metrics.touched_paths > self.thresholds.deny_touched_paths {
            return Some(self.deny(
                metrics,
                DenyReason::TouchedPathLimit {
                    limit: self.thresholds.deny_touched_paths,
                    observed: metrics.touched_paths,
                },
            ));
        }
        if metrics.changed_bytes > self.thresholds.deny_changed_bytes {
            return Some(self.deny(
                metrics,
                DenyReason::ChangedByteLimit {
                    limit: self.thresholds.deny_changed_bytes,
                    observed: metrics.changed_bytes,
                },
            ));
        }
        if metrics.deleted_paths > self.thresholds.deny_deleted_paths {
            return Some(self.deny(
                metrics,
                DenyReason::DeletePathLimit {
                    limit: self.thresholds.deny_deleted_paths,
                    observed: metrics.deleted_paths,
                },
            ));
        }
        if metrics.deleted_paths >= self.thresholds.delete_ratio_minimum_paths
            && metrics.delete_ratio_bps >= self.thresholds.deny_delete_ratio_bps
        {
            return Some(self.deny(
                metrics,
                DenyReason::DeleteRatioLimit {
                    limit_bps: self.thresholds.deny_delete_ratio_bps,
                    observed_bps: metrics.delete_ratio_bps,
                },
            ));
        }
        None
    }

    fn risk_flags(&self, metrics: RiskMetrics) -> BTreeSet<RiskFlag> {
        let mut flags = BTreeSet::new();
        if matches!(
            self.profile,
            PolicyProfile::Strict | PolicyProfile::Paranoid
        ) {
            flags.insert(RiskFlag::Mutation);
        }
        if metrics.deleted_paths > 0 {
            flags.insert(RiskFlag::Deletion);
        }
        if metrics.renamed_paths > 0 {
            flags.insert(RiskFlag::Rename);
        }
        if metrics.executable_changes > 0 {
            flags.insert(RiskFlag::ExecutableChange);
        }
        if metrics.symlink_changes > 0 {
            flags.insert(RiskFlag::SymlinkChange);
        }
        if metrics.touched_paths >= self.thresholds.escalate_touched_paths {
            flags.insert(RiskFlag::LargeTouchedSet);
        }
        if metrics.changed_bytes >= self.thresholds.escalate_changed_bytes {
            flags.insert(RiskFlag::LargeByteChange);
        }
        flags
    }

    fn deny(&self, metrics: RiskMetrics, reason: DenyReason) -> PolicyDecision {
        PolicyDecision::Deny(DenyManifest {
            reason,
            metrics,
            policy: self.digest,
        })
    }
}

impl Default for TransactionPolicy {
    fn default() -> Self {
        Self::preset(PolicyProfile::Balanced)
    }
}

fn validate_thresholds(thresholds: PolicyThresholds) -> Result<(), PolicyConfigError> {
    if thresholds.escalate_touched_paths == 0
        || thresholds.deny_touched_paths == 0
        || thresholds.escalate_touched_paths > thresholds.deny_touched_paths
    {
        return Err(PolicyConfigError::TouchedPathThreshold);
    }
    if thresholds.escalate_changed_bytes == 0
        || thresholds.deny_changed_bytes == 0
        || thresholds.escalate_changed_bytes > thresholds.deny_changed_bytes
    {
        return Err(PolicyConfigError::ChangedByteThreshold);
    }
    if thresholds.deny_deleted_paths == 0 || thresholds.delete_ratio_minimum_paths == 0 {
        return Err(PolicyConfigError::DeleteThreshold);
    }
    if !(1..=10_000).contains(&thresholds.deny_delete_ratio_bps) {
        return Err(PolicyConfigError::DeleteRatio);
    }
    Ok(())
}

fn policy_digest(
    profile: PolicyProfile,
    thresholds: PolicyThresholds,
    call_policy: &CallPolicy,
) -> PolicyDigest {
    let mut canonical = Vec::new();
    encode_bytes(POLICY_SCHEMA_VERSION.as_bytes(), &mut canonical);
    canonical.push(profile.tag());
    canonical.extend_from_slice(&(thresholds.escalate_touched_paths as u64).to_le_bytes());
    canonical.extend_from_slice(&thresholds.escalate_changed_bytes.to_le_bytes());
    canonical.extend_from_slice(&(thresholds.deny_touched_paths as u64).to_le_bytes());
    canonical.extend_from_slice(&thresholds.deny_changed_bytes.to_le_bytes());
    canonical.extend_from_slice(&(thresholds.deny_deleted_paths as u64).to_le_bytes());
    canonical.extend_from_slice(&(thresholds.delete_ratio_minimum_paths as u64).to_le_bytes());
    canonical.extend_from_slice(&thresholds.deny_delete_ratio_bps.to_le_bytes());
    call_policy.encode_canonical(&mut canonical);
    PolicyDigest::digest_canonical(&canonical)
}

/// Inputs observed by deterministic transaction policy.
#[derive(Clone, Copy)]
pub struct PolicyInput<'a> {
    /// Exact canonical virtual diff.
    pub diff: &'a CanonicalDiff,
    /// Operation-local observations, including rename semantics.
    pub effects: &'a [EffectEvent],
    /// Attempts denied before VFS access, including attempts caught by the program.
    pub denied_accesses: &'a [DeniedAccess],
    /// Number of base nodes including the virtual root.
    pub base_node_count: usize,
}

/// Exact bounded metrics used for a deterministic policy decision.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RiskMetrics {
    /// All canonical changed paths.
    pub touched_paths: usize,
    /// Newly created paths.
    pub created_paths: usize,
    /// Content or metadata modifications.
    pub modified_paths: usize,
    /// Deleted paths, including recursive-delete closure.
    pub deleted_paths: usize,
    /// Semantic rename operations observed in the ledger.
    pub renamed_paths: usize,
    /// Sum of before and after bytes represented by changed non-directory nodes.
    pub changed_bytes: u64,
    /// Deletions divided by user-visible base nodes, in basis points.
    pub delete_ratio_bps: u16,
    /// Changes that add/remove an executable mode bit.
    pub executable_changes: usize,
    /// Changes involving an opaque symbolic link.
    pub symlink_changes: usize,
}

impl RiskMetrics {
    /// Derive the exact metrics used by policy from bounded transaction evidence.
    #[must_use]
    pub fn from_evidence(
        diff: &CanonicalDiff,
        effects: &[EffectEvent],
        base_node_count: usize,
    ) -> Self {
        let mut metrics = Self {
            touched_paths: diff.entries().len(),
            ..Self::default()
        };
        for entry in diff.entries() {
            match entry.kind {
                DiffKind::Create => metrics.created_paths += 1,
                DiffKind::Delete => metrics.deleted_paths += 1,
                DiffKind::Modify | DiffKind::MetadataChange => metrics.modified_paths += 1,
            }
            metrics.changed_bytes = metrics
                .changed_bytes
                .saturating_add(entry.before.map_or(0, NodeState::size))
                .saturating_add(entry.after.map_or(0, NodeState::size));
            let before_executable = entry.before.is_some_and(|state| state.mode() & 0o111 != 0);
            let after_executable = entry.after.is_some_and(|state| state.mode() & 0o111 != 0);
            if before_executable != after_executable {
                metrics.executable_changes += 1;
            }
            if entry
                .before
                .is_some_and(|state| state.kind() == NodeKind::Symlink)
                || entry
                    .after
                    .is_some_and(|state| state.kind() == NodeKind::Symlink)
            {
                metrics.symlink_changes += 1;
            }
        }
        metrics.renamed_paths = effects
            .iter()
            .filter(|event| matches!(event.effect, Effect::Rename { .. }))
            .count();
        let base_user_nodes = base_node_count.saturating_sub(1);
        let numerator = metrics.deleted_paths.saturating_mul(10_000);
        let ratio = numerator.checked_div(base_user_nodes).unwrap_or(0);
        metrics.delete_ratio_bps = u16::try_from(ratio.min(10_000)).unwrap_or(10_000);
        metrics
    }
}

/// Stable reason a deterministic policy must reject a transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DenyReason {
    /// Sandboxed code attempted a protected capability, even if it caught the error.
    ProtectedAccessAttempt(DeniedAccess),
    /// Final state changed a protected path through a non-Monty/core caller.
    ProtectedMutation(DeniedAccess),
    /// Canonical touched-path count exceeded the hard ceiling.
    TouchedPathLimit {
        /// Configured hard ceiling.
        limit: usize,
        /// Observed count.
        observed: usize,
    },
    /// Changed bytes exceeded the hard ceiling.
    ChangedByteLimit {
        /// Configured hard ceiling.
        limit: u64,
        /// Observed count.
        observed: u64,
    },
    /// Deleted paths exceeded the hard ceiling.
    DeletePathLimit {
        /// Configured hard ceiling.
        limit: usize,
        /// Observed count.
        observed: usize,
    },
    /// A sufficiently large deletion exceeded the workspace-ratio ceiling.
    DeleteRatioLimit {
        /// Configured ceiling in basis points.
        limit_bps: u16,
        /// Observed ratio in basis points.
        observed_bps: u16,
    },
}

/// Why an otherwise valid transaction requires an independent approval principal.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RiskFlag {
    /// The selected profile escalates every mutation.
    Mutation,
    /// One or more paths are deleted.
    Deletion,
    /// A semantic rename was observed.
    Rename,
    /// Executable mode changed.
    ExecutableChange,
    /// An opaque symbolic link is created, removed, or replaced.
    SymlinkChange,
    /// Touched-path escalation threshold was reached.
    LargeTouchedSet,
    /// Changed-byte escalation threshold was reached.
    LargeByteChange,
}

/// Deterministic denial payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DenyManifest {
    /// Stable deterministic reason.
    pub reason: DenyReason,
    /// Exact metrics evaluated.
    pub metrics: RiskMetrics,
    /// Exact policy configuration digest.
    pub policy: PolicyDigest,
}

/// Bounded evidence shown to a fresh approval principal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskManifest {
    /// Exact metrics evaluated.
    pub metrics: RiskMetrics,
    /// Stable, sorted risk flags.
    pub flags: Vec<RiskFlag>,
    /// Exact policy configuration digest.
    pub policy: PolicyDigest,
}

/// Final deterministic transaction decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyDecision {
    /// Hard policy rejected the transaction; no judge may reverse it.
    Deny(DenyManifest),
    /// Deterministic rules authorize reservation without a judge.
    AutoApprove,
    /// An independent fresh judge or human must narrow the decision.
    Escalate(RiskManifest),
}

/// Canonically hash a read dependency set.
#[must_use]
pub fn read_set_digest(read_set: &BTreeMap<VPath, ReadObservation>) -> ReadSetDigest {
    let mut canonical = Vec::new();
    encode_usize(read_set.len(), &mut canonical);
    for (path, observation) in read_set {
        encode_path(path, &mut canonical);
        match observation.metadata {
            None => canonical.push(0),
            Some(None) => canonical.push(1),
            Some(Some(state)) => {
                canonical.push(2);
                state.encode_canonical(&mut canonical);
            }
        }
        encode_optional_digest(
            observation.content.map(|digest| *digest.as_bytes()),
            &mut canonical,
        );
        encode_optional_digest(
            observation.directory.map(|digest| *digest.as_bytes()),
            &mut canonical,
        );
    }
    ReadSetDigest::digest_canonical(&canonical)
}

/// Canonically hash write preconditions.
#[must_use]
pub fn write_set_digest(write_set: &BTreeMap<VPath, WritePrecondition>) -> WriteSetDigest {
    let mut canonical = Vec::new();
    encode_usize(write_set.len(), &mut canonical);
    for (path, precondition) in write_set {
        encode_path(path, &mut canonical);
        encode_optional_state(precondition.expected, &mut canonical);
    }
    WriteSetDigest::digest_canonical(&canonical)
}

/// Inputs used to construct an approval-bound transaction identity.
#[derive(Clone, Copy)]
pub struct TransactionIdentityInput<'a> {
    /// Immutable base snapshot.
    pub base_snapshot: SnapshotId,
    /// Exact canonical diff.
    pub diff: &'a CanonicalDiff,
    /// Exact read dependencies.
    pub read_set: &'a BTreeMap<VPath, ReadObservation>,
    /// Exact write preconditions.
    pub write_set: &'a BTreeMap<VPath, WritePrecondition>,
    /// Exact program source.
    pub program: &'a str,
    /// Exact deterministic policy configuration.
    pub policy: &'a TransactionPolicy,
    /// Canonical security-relevant execution configuration.
    pub runtime_config: RuntimeConfigDigest,
    /// Optional original intent carried out of band.
    pub intent: Option<&'a str>,
}

/// Bind every approval-relevant artifact into one immutable transaction identity.
#[must_use]
pub fn bind_transaction(input: TransactionIdentityInput<'_>) -> TransactionBinding {
    TransactionBinding {
        base_snapshot: input.base_snapshot,
        diff: input.diff.digest(),
        read_set: read_set_digest(input.read_set),
        write_set: write_set_digest(input.write_set),
        program: ProgramDigest::digest_source(input.program),
        policy: input.policy.digest(),
        runtime_config: input.runtime_config,
        intent: input.intent.map(IntentDigest::digest_text),
    }
}

fn encode_usize(value: usize, output: &mut Vec<u8>) {
    output.extend_from_slice(&(value as u64).to_le_bytes());
}

fn encode_bytes(bytes: &[u8], output: &mut Vec<u8>) {
    output.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    output.extend_from_slice(bytes);
}

fn encode_path(path: &VPath, output: &mut Vec<u8>) {
    encode_bytes(path.as_str().as_bytes(), output);
}

fn encode_optional_state(state: Option<NodeState>, output: &mut Vec<u8>) {
    match state {
        Some(state) => {
            output.push(1);
            state.encode_canonical(output);
        }
        None => output.push(0),
    }
}

fn encode_optional_digest(digest: Option<[u8; 32]>, output: &mut Vec<u8>) {
    match digest {
        Some(digest) => {
            output.push(1);
            output.extend_from_slice(&digest);
        }
        None => output.push(0),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use vsh_store::BlobStore;
    use vsh_types::{RuntimeConfigDigest, VPath};
    use vsh_vfs::{SnapshotBuilder, VirtualFs};

    use super::*;

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("vsh-policy-test-{}-{sequence}", std::process::id()));
            fs::create_dir(&path).expect("test directory should be unique");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn filesystem(files: &[(&str, &[u8])]) -> (TestDirectory, VirtualFs) {
        let directory = TestDirectory::new();
        let store = BlobStore::open(directory.path()).unwrap();
        let mut builder = SnapshotBuilder::new(store);
        for (path, bytes) in files {
            if let Some(parent) = VPath::parse(path).unwrap().parent()
                && !parent.is_root()
            {
                let _ = builder.add_directory(parent, 0o755);
            }
            builder
                .add_file(VPath::parse(path).unwrap(), bytes, 0o644)
                .unwrap();
        }
        let snapshot = builder.build().unwrap();
        (directory, VirtualFs::new(snapshot))
    }

    #[test]
    fn cursor_globstar_matches_dynamic_programming_oracle() {
        fn sequences<'a>(alphabet: &[&'a str], depth: usize) -> Vec<Vec<&'a str>> {
            let mut sequences = vec![Vec::new()];
            let mut frontier = vec![Vec::new()];
            for _ in 0..depth {
                frontier = frontier
                    .iter()
                    .flat_map(|prefix| {
                        alphabet.iter().map(move |component| {
                            let mut next = prefix.clone();
                            next.push(*component);
                            next
                        })
                    })
                    .collect();
                sequences.extend(frontier.iter().cloned());
            }
            sequences
        }

        // The original matcher is deliberately retained as an independent
        // oracle. Include empty paths, consecutive and separated globstars,
        // failed suffix retries, basename wildcards and UTF-8 components.
        fn oracle(pattern: &[String], path: &[&str]) -> bool {
            let mut previous = vec![false; path.len() + 1];
            previous[0] = true;
            for component in pattern {
                let mut current = vec![false; path.len() + 1];
                if component == "**" {
                    current[0] = previous[0];
                    for index in 1..current.len() {
                        current[index] = previous[index] || current[index - 1];
                    }
                } else {
                    for index in 1..current.len() {
                        current[index] =
                            previous[index - 1] && component_matches(component, path[index - 1]);
                    }
                }
                previous = current;
            }
            previous[path.len()]
        }

        let paths = sequences(&["a", "b", "é"], 4);
        for components in sequences(&["a", "b", "*", "a*", "**"], 4) {
            let pattern: Vec<String> = components.iter().map(ToString::to_string).collect();
            let compiled = (!components.is_empty())
                .then(|| PathPattern::compile(components.join("/")).unwrap());
            for path in &paths {
                assert_eq!(
                    path_components_match(&pattern, &path.join("/")),
                    oracle(&pattern, path),
                    "pattern {pattern:?}, path {path:?}"
                );
                if let Some(compiled) = &compiled {
                    let expected = if pattern.len() == 1 && pattern[0] != "**" {
                        path.last()
                            .is_some_and(|name| component_matches(&pattern[0], name))
                    } else {
                        oracle(&pattern, path)
                    };
                    let virtual_path =
                        VPath::parse(&path.join("/")).unwrap_or_else(|_| VPath::root());
                    assert_eq!(
                        compiled.matches(&virtual_path),
                        expected,
                        "{pattern:?} {path:?}"
                    );
                }
            }
        }
        let deep = vec!["a"; 4096].join("/");
        assert!(path_components_match(&["**".into(), "a".into()], &deep));
        assert!(!path_components_match(&["**".into(), "b".into()], &deep));
    }

    #[test]
    fn pattern_fast_paths_preserve_root_and_normalization() {
        for (pattern, path, expected) in [
            ("*", ".", false),
            ("**", ".", true),
            ("**/**", ".", true),
            ("a/**", ".", false),
            ("./a//**/b/", "a/b", true),
            ("**/é*", "a/été", true),
            ("**/**/*.key", "a/b/private.key", true),
            ("**/*.key", ".", false),
            ("**/a/**/b", "a/x/a/y/b", true),
            ("**/a/**/b", "a/x/a/y/c", false),
            ("*.key", "a/private.key", true),
            ("a/*", "a/b/c", false),
        ] {
            assert_eq!(
                PathPattern::compile(pattern)
                    .unwrap()
                    .matches(&VPath::parse(path).unwrap()),
                expected,
                "pattern {pattern}, path {path}"
            );
        }
    }

    #[test]
    fn portable_patterns_cover_root_nested_and_subtree_secrets() {
        let policy = CallPolicy::default();
        for path in [
            ".env",
            ".env/token",
            "app/.env.local",
            "app/.env.local/token",
            "secrets/token.txt",
            "nested/private.key",
            "deploy/id_rsa",
            "a/credentials.json",
        ] {
            assert!(
                policy
                    .authorize(&VPath::parse(path).unwrap(), AccessKind::ContentRead)
                    .is_err(),
                "path should be protected: {path}"
            );
        }
        assert!(
            policy
                .authorize(
                    &VPath::parse("src/main.rs").unwrap(),
                    AccessKind::ContentRead
                )
                .is_ok()
        );
    }

    #[test]
    fn git_reads_are_allowed_but_mutations_are_denied() {
        let policy = CallPolicy::default();
        let path = VPath::parse(".git/config").unwrap();
        assert!(policy.authorize(&path, AccessKind::ContentRead).is_ok());
        let denial = policy.authorize(&path, AccessKind::Modify).unwrap_err();
        assert_eq!(denial.rule, ".git/**");
    }

    #[test]
    fn malformed_patterns_fail_closed() {
        for (pattern, expected) in [
            ("", PatternError::Empty),
            ("/secret", PatternError::Absolute),
            ("a\\b", PatternError::Backslash),
            ("../secret", PatternError::ParentComponent),
            ("foo**bar", PatternError::InvalidGlobstar),
            ("secret?.txt", PatternError::UnsupportedMetacharacter),
        ] {
            assert_eq!(
                ProtectedRule::new(pattern, AccessSet::ALL).unwrap_err(),
                expected
            );
        }
    }

    #[test]
    fn balanced_auto_approves_small_non_destructive_edit() {
        let (_guard, mut filesystem) = filesystem(&[("input.txt", b"one")]);
        filesystem
            .write(&VPath::parse("output.txt").unwrap(), b"two")
            .unwrap();
        let diff = filesystem.canonical_diff().unwrap();
        let policy = TransactionPolicy::default();
        let decision = policy.evaluate(PolicyInput {
            diff: &diff,
            effects: filesystem.effects(),
            denied_accesses: &[],
            base_node_count: 2,
        });
        assert_eq!(decision, PolicyDecision::AutoApprove);
    }

    #[test]
    fn strict_escalates_small_mutation_without_mislabeling_it_as_large() {
        let (_guard, mut filesystem) = filesystem(&[]);
        filesystem
            .write(&VPath::parse("output.txt").unwrap(), b"two")
            .unwrap();
        let diff = filesystem.canonical_diff().unwrap();
        let decision = TransactionPolicy::preset(PolicyProfile::Strict).evaluate(PolicyInput {
            diff: &diff,
            effects: filesystem.effects(),
            denied_accesses: &[],
            base_node_count: 1,
        });
        let PolicyDecision::Escalate(manifest) = decision else {
            panic!("strict mutation should escalate")
        };
        assert_eq!(manifest.flags, vec![RiskFlag::Mutation]);
    }

    #[test]
    fn balanced_escalates_delete_and_rename() {
        let (_guard, mut filesystem) = filesystem(&[("input.txt", b"one")]);
        filesystem
            .rename(
                &VPath::parse("input.txt").unwrap(),
                &VPath::parse("archive.txt").unwrap(),
            )
            .unwrap();
        let diff = filesystem.canonical_diff().unwrap();
        let decision = TransactionPolicy::default().evaluate(PolicyInput {
            diff: &diff,
            effects: filesystem.effects(),
            denied_accesses: &[],
            base_node_count: 2,
        });
        let PolicyDecision::Escalate(manifest) = decision else {
            panic!("rename should escalate")
        };
        assert_eq!(manifest.metrics.deleted_paths, 1);
        assert_eq!(manifest.metrics.renamed_paths, 1);
        assert_eq!(manifest.flags, vec![RiskFlag::Deletion, RiskFlag::Rename]);
    }

    #[test]
    fn caught_protected_attempt_forces_final_deny() {
        let (_guard, filesystem) = filesystem(&[]);
        let diff = filesystem.canonical_diff().unwrap();
        let attempt = DeniedAccess {
            path: VPath::parse(".env").unwrap(),
            access: AccessKind::ContentRead,
            rule: ".env".to_owned(),
        };
        let decision = TransactionPolicy::default().evaluate(PolicyInput {
            diff: &diff,
            effects: filesystem.effects(),
            denied_accesses: std::slice::from_ref(&attempt),
            base_node_count: 1,
        });
        assert!(matches!(
            decision,
            PolicyDecision::Deny(DenyManifest {
                reason: DenyReason::ProtectedAccessAttempt(ref denied),
                ..
            }) if denied == &attempt
        ));
    }

    #[test]
    fn final_policy_rechecks_protected_mutations() {
        let (_guard, mut filesystem) = filesystem(&[]);
        filesystem
            .write(&VPath::parse(".env").unwrap(), b"secret")
            .unwrap();
        let diff = filesystem.canonical_diff().unwrap();
        let decision = TransactionPolicy::default().evaluate(PolicyInput {
            diff: &diff,
            effects: filesystem.effects(),
            denied_accesses: &[],
            base_node_count: 1,
        });
        assert!(matches!(
            decision,
            PolicyDecision::Deny(DenyManifest {
                reason: DenyReason::ProtectedMutation(_),
                ..
            })
        ));
    }

    #[test]
    fn transaction_identity_changes_with_every_bound_context() {
        let (_guard, mut filesystem) = filesystem(&[("input.txt", b"one")]);
        filesystem
            .read(&VPath::parse("input.txt").unwrap())
            .unwrap();
        filesystem
            .write(&VPath::parse("output.txt").unwrap(), b"two")
            .unwrap();
        let diff = filesystem.canonical_diff().unwrap();
        let policy = TransactionPolicy::default();
        let runtime_config = RuntimeConfigDigest::digest_canonical(b"limits-a");
        let first = bind_transaction(TransactionIdentityInput {
            base_snapshot: SnapshotId::from_bytes([1; 32]),
            diff: &diff,
            read_set: filesystem.read_set(),
            write_set: filesystem.write_set(),
            program: "program-a",
            policy: &policy,
            runtime_config,
            intent: Some("intent-a"),
        });
        let second = bind_transaction(TransactionIdentityInput {
            program: "program-b",
            ..TransactionIdentityInput {
                base_snapshot: SnapshotId::from_bytes([1; 32]),
                diff: &diff,
                read_set: filesystem.read_set(),
                write_set: filesystem.write_set(),
                program: "program-a",
                policy: &policy,
                runtime_config,
                intent: Some("intent-a"),
            }
        });
        assert_ne!(first.transaction_id(), second.transaction_id());
        assert_eq!(
            first.transaction_id(),
            bind_transaction(TransactionIdentityInput {
                base_snapshot: SnapshotId::from_bytes([1; 32]),
                diff: &diff,
                read_set: filesystem.read_set(),
                write_set: filesystem.write_set(),
                program: "program-a",
                policy: &policy,
                runtime_config,
                intent: Some("intent-a"),
            })
            .transaction_id()
        );
    }

    #[test]
    fn policy_configuration_errors_have_distinct_stable_messages() {
        let pattern_errors = [
            PatternError::Empty,
            PatternError::Absolute,
            PatternError::Backslash,
            PatternError::NulByte,
            PatternError::ParentComponent,
            PatternError::InvalidGlobstar,
            PatternError::UnsupportedMetacharacter,
        ];
        assert_eq!(
            pattern_errors
                .map(|error| error.to_string())
                .into_iter()
                .collect::<BTreeSet<_>>()
                .len(),
            pattern_errors.len()
        );

        let config_errors = [
            PolicyConfigError::TouchedPathThreshold,
            PolicyConfigError::ChangedByteThreshold,
            PolicyConfigError::DeleteThreshold,
            PolicyConfigError::DeleteRatio,
        ];
        assert_eq!(
            config_errors
                .map(|error| error.to_string())
                .into_iter()
                .collect::<BTreeSet<_>>()
                .len(),
            config_errors.len()
        );

        for access in [
            AccessKind::MetadataRead,
            AccessKind::ContentRead,
            AccessKind::DirectoryRead,
        ] {
            assert!(!access.is_mutation());
        }
        for access in [
            AccessKind::Create,
            AccessKind::Modify,
            AccessKind::Delete,
            AccessKind::RenameSource,
            AccessKind::RenameDestination,
        ] {
            assert!(access.is_mutation());
            assert!(AccessSet::MUTATIONS.contains(access));
        }
        assert!(!AccessSet::NONE.contains(AccessKind::Create));
    }
}
