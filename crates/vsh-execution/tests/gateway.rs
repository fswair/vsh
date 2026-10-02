//! Characterization of the shared filesystem authority and its failure boundaries.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use vsh_execution::{ExecutionBudget, ExecutionLimits, FsGateway, GatewayError, OpenMode};
use vsh_policy::{AccessKind, AccessSet, CallPolicy, ProtectedRule};
use vsh_store::BlobStore;
use vsh_types::VPath;
use vsh_vfs::{BaseSnapshot, Effect, EffectOrigin, SnapshotBuilder, VfsError, VirtualFs};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    directory: PathBuf,
    base: Option<BaseSnapshot>,
}

impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "vsh-gateway-test-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&directory).unwrap();
        let mut builder = SnapshotBuilder::new(BlobStore::open(&directory).unwrap());
        builder
            .add_file(VPath::parse("data.bin").unwrap(), &[0xff, 0, 0xfe], 0o600)
            .unwrap();
        builder
            .add_directory(VPath::parse("folder").unwrap(), 0o755)
            .unwrap();
        builder
            .add_file(VPath::parse("folder/visible.txt").unwrap(), b"hello", 0o644)
            .unwrap();
        builder
            .add_file(VPath::parse("folder/secret.txt").unwrap(), b"secret", 0o600)
            .unwrap();
        builder
            .add_symlink(VPath::parse("link").unwrap(), b"/etc/passwd", 0o777)
            .unwrap();
        Self {
            directory,
            base: Some(builder.build().unwrap()),
        }
    }

    fn filesystem(&self) -> VirtualFs {
        VirtualFs::new(self.base.as_ref().unwrap().clone())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Windows keeps the data directory pinned while the snapshot owns its
        // blob-store capability. Release it before removing fixture files.
        drop(self.base.take());
        fs::remove_dir_all(&self.directory).unwrap();
    }
}

#[test]
fn fixture_releases_snapshot_handles_before_removing_its_directory() {
    let fixture = Fixture::new();
    let directory = fixture.directory.clone();
    drop(fixture);
    assert!(!directory.exists());
}

#[test]
fn recursive_copy_reserves_long_destinations_during_preflight_before_any_mutation() {
    let fixture = Fixture::new();
    let mut builder = SnapshotBuilder::new(BlobStore::open(&fixture.directory).unwrap());
    let source = VPath::parse("src").unwrap();
    let destination = VPath::parse(&"x".repeat(8192)).unwrap();
    builder.add_directory(source.clone(), 0o755).unwrap();
    for index in 0..20 {
        builder
            .add_file(
                VPath::parse(&format!("src/{index:04}")).unwrap(),
                b"",
                0o644,
            )
            .unwrap();
    }
    let mut filesystem = VirtualFs::new(builder.build().unwrap());
    let policy = CallPolicy::default();
    let mut budget = ExecutionBudget::new(ExecutionLimits {
        max_evidence_bytes: 64 * 1024,
        ..ExecutionLimits::default()
    });
    budget.charge_os_call().unwrap();
    let result = FsGateway::new(
        &mut filesystem,
        &policy,
        &mut budget,
        EffectOrigin::MontyToolCall,
    )
    .copy(&source, &destination, true, false);
    assert!(matches!(
        result,
        Err(GatewayError::Limit(
            vsh_execution::ExecutionLimitExceeded::EvidenceBytes { limit: 65536, .. }
        ))
    ));
    assert_eq!(filesystem.metrics().overlay_entries, 0);
    assert!(filesystem.write_set().is_empty());
    assert_eq!(budget.stats().os_calls, 1);
    assert_eq!(budget.stats().read_bytes, 0);
    assert_eq!(budget.stats().write_bytes, 0);
    assert_eq!(budget.stats().directory_entries, 20);
    assert!(filesystem.evidence_usage().bytes <= 65536);
    assert!(filesystem.canonical_diff().is_err());
}

#[test]
fn retained_denials_share_the_cap_without_recording_fake_filesystem_effects() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let path = VPath::parse(&format!("blocked/{}", "x".repeat(8192))).unwrap();
    let policy = CallPolicy::new(vec![
        ProtectedRule::new("blocked/**", AccessSet::CONTENT_READ).unwrap(),
    ]);
    let limits = ExecutionLimits {
        max_evidence_records: 2,
        ..ExecutionLimits::default()
    };
    let mut denied = Vec::new();
    for _ in 0..2 {
        let denial = policy
            .authorize(&path, AccessKind::ContentRead)
            .unwrap_err();
        vsh_execution::retain_denial(&mut filesystem, limits, &mut denied, denial).unwrap();
    }
    let denial = policy
        .authorize(&path, AccessKind::ContentRead)
        .unwrap_err();
    assert!(matches!(
        vsh_execution::retain_denial(&mut filesystem, limits, &mut denied, denial),
        Err(GatewayError::Limit(
            vsh_execution::ExecutionLimitExceeded::EvidenceRecords {
                limit: 2,
                attempted: 3
            }
        ))
    ));
    assert_eq!(denied.len(), 2);
    assert_eq!(filesystem.evidence_usage().records, 2);
    assert!(filesystem.read_set().is_empty());
    assert!(filesystem.effects().is_empty());
    assert!(filesystem.canonical_diff().is_err());
}

#[test]
fn recursive_mkdir_reserves_its_frontier_before_cloning_or_observing_ancestors() {
    let fixture = Fixture::new();
    let path = VPath::parse(&format!("{}/child", "x".repeat(8192))).unwrap();
    let mut observed = fixture.filesystem();
    assert!(matches!(
        observed.metadata(&path),
        Err(VfsError::NotFound { .. })
    ));
    let first_buffer = u64::try_from(4 * size_of::<VPath>() + path.as_str().len()).unwrap();
    let limit = observed.evidence_usage().bytes + first_buffer - 1;
    let mut filesystem = fixture.filesystem();
    let policy = CallPolicy::default();
    let mut budget = ExecutionBudget::new(ExecutionLimits {
        max_evidence_bytes: limit,
        ..ExecutionLimits::default()
    });
    budget.charge_os_call().unwrap();
    assert!(matches!(
        FsGateway::new(
            &mut filesystem,
            &policy,
            &mut budget,
            EffectOrigin::BashCall
        )
        .mkdir_options(&path, 0o755, true, false),
        Err(GatewayError::Limit(
            vsh_execution::ExecutionLimitExceeded::EvidenceBytes { limit: actual, .. }
        )) if actual == limit
    ));
    assert_eq!(filesystem.read_set().len(), 1);
    assert!(filesystem.read_set().contains_key(&path));
    assert!(filesystem.write_set().is_empty());
    assert_eq!(filesystem.metrics().overlay_entries, 0);
    assert_eq!(budget.stats().os_calls, 1);
    assert!(filesystem.canonical_diff().is_err());
}

#[test]
fn denial_path_and_rule_bytes_are_reserved_before_retention() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let path = VPath::parse(&format!("blocked/{}", "x".repeat(8192))).unwrap();
    let policy = CallPolicy::new(vec![
        ProtectedRule::new("blocked/**", AccessSet::CONTENT_READ).unwrap(),
    ]);
    let denial = policy
        .authorize(&path, AccessKind::ContentRead)
        .unwrap_err();
    let limit = u64::try_from(
        2 * size_of::<vsh_policy::DeniedAccess>() + path.as_str().len() + denial.rule.len(),
    )
    .unwrap();
    let limits = ExecutionLimits {
        max_evidence_bytes: limit,
        ..ExecutionLimits::default()
    };
    let mut denied = Vec::new();
    vsh_execution::retain_denial(&mut filesystem, limits, &mut denied, denial).unwrap();
    let denial = policy
        .authorize(&path, AccessKind::ContentRead)
        .unwrap_err();
    assert!(matches!(
        vsh_execution::retain_denial(&mut filesystem, limits, &mut denied, denial),
        Err(GatewayError::Limit(
            vsh_execution::ExecutionLimitExceeded::EvidenceBytes { limit: actual, .. }
        )) if actual == limit
    ));
    assert_eq!(denied.len(), 1);
    assert_eq!(filesystem.evidence_usage().records, 1);
    assert_eq!(filesystem.evidence_usage().bytes, limit);
    assert!(filesystem.read_set().is_empty());
    assert!(filesystem.effects().is_empty());
    assert!(filesystem.canonical_diff().is_err());
}

#[test]
fn active_evidence_limits_are_shared_sticky_and_do_not_fabricate_io_counters() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let policy = CallPolicy::default();
    let path = VPath::parse(&"x".repeat(8192)).unwrap();
    let mut budget = ExecutionBudget::new(ExecutionLimits {
        max_evidence_records: 3,
        ..ExecutionLimits::default()
    });
    budget.charge_os_call().unwrap();
    assert!(
        matches!(FsGateway::new(&mut filesystem, &policy, &mut budget, EffectOrigin::MontyOsCall).metadata(&path), Err(GatewayError::Filesystem(source)) if matches!(*source, VfsError::NotFound { .. }))
    );
    budget.charge_os_call().unwrap();
    let _ = FsGateway::new(
        &mut filesystem,
        &policy,
        &mut budget,
        EffectOrigin::BashCall,
    )
    .metadata(&path);
    budget.charge_os_call().unwrap();
    assert!(matches!(
        FsGateway::new(
            &mut filesystem,
            &policy,
            &mut budget,
            EffectOrigin::BashCall
        )
        .metadata(&path),
        Err(GatewayError::Limit(
            vsh_execution::ExecutionLimitExceeded::EvidenceRecords {
                limit: 3,
                attempted: 4
            }
        ))
    ));
    assert_eq!(budget.stats().os_calls, 3);
    assert_eq!(budget.stats().read_bytes, 0);
    assert_eq!(budget.stats().write_bytes, 0);
    assert_eq!(budget.stats().directory_entries, 0);
    assert_eq!(filesystem.effects().len(), 2);
    assert_eq!(filesystem.read_set().len(), 1);
    assert_eq!(filesystem.effects()[0].origin, EffectOrigin::MontyOsCall);
    assert_eq!(filesystem.effects()[1].origin, EffectOrigin::BashCall);
    let mut replacement_budget = ExecutionBudget::new(ExecutionLimits::default());
    assert!(matches!(
        FsGateway::new(
            &mut filesystem,
            &policy,
            &mut replacement_budget,
            EffectOrigin::VirtualFs
        )
        .metadata(&VPath::root()),
        Err(GatewayError::Limit(
            vsh_execution::ExecutionLimitExceeded::EvidenceRecords { .. }
        ))
    ));
    assert!(filesystem.canonical_diff().is_err());
}

#[test]
fn evidence_byte_limit_is_terminal_before_content_loading_or_mutation() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let policy = CallPolicy::default();
    let mut budget = ExecutionBudget::new(ExecutionLimits {
        max_evidence_bytes: 0,
        ..ExecutionLimits::default()
    });
    assert!(matches!(
        FsGateway::new(
            &mut filesystem,
            &policy,
            &mut budget,
            EffectOrigin::BashCall
        )
        .read(&VPath::parse("data.bin").unwrap()),
        Err(GatewayError::Limit(
            vsh_execution::ExecutionLimitExceeded::EvidenceBytes { limit: 0, .. }
        ))
    ));
    assert_eq!(budget.stats().read_bytes, 0);
    assert!(filesystem.read_set().is_empty());
    assert!(filesystem.effects().is_empty());
    assert!(filesystem.canonical_diff().is_err());
}

#[test]
fn binary_read_matches_existing_observations_and_effects() {
    let fixture = Fixture::new();
    let mut actual = fixture.filesystem();
    let mut expected = fixture.filesystem();
    let path = VPath::parse("data.bin").unwrap();
    let policy = CallPolicy::default();
    let mut budget = ExecutionBudget::new(ExecutionLimits::default());
    let bytes = FsGateway::new(&mut actual, &policy, &mut budget, EffectOrigin::MontyOsCall)
        .read(&path)
        .unwrap();
    expected.with_effect_origin(EffectOrigin::MontyOsCall, |filesystem| {
        filesystem.metadata(&path).unwrap();
        assert_eq!(filesystem.read(&path).unwrap(), bytes);
    });
    assert_eq!(bytes, [0xff, 0, 0xfe]);
    assert_eq!(actual.effects(), expected.effects());
    assert_eq!(actual.read_set(), expected.read_set());
    assert_eq!(actual.write_set(), expected.write_set());
    assert_eq!(budget.stats().read_bytes, 3);
    assert_eq!(actual.effect_origin(), EffectOrigin::VirtualFs);
}

#[test]
fn denied_read_does_not_observe_content_or_metadata() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let path = VPath::parse("data.bin").unwrap();
    let policy = CallPolicy::new(vec![
        ProtectedRule::new("data.bin", AccessSet::CONTENT_READ).unwrap(),
    ]);
    let mut budget = ExecutionBudget::new(ExecutionLimits::default());
    let result = FsGateway::new(
        &mut filesystem,
        &policy,
        &mut budget,
        EffectOrigin::MontyToolCall,
    )
    .read(&path);
    assert!(
        matches!(result, Err(GatewayError::Policy(denial)) if denial.access == AccessKind::ContentRead)
    );
    assert!(filesystem.effects().is_empty());
    assert!(filesystem.read_set().is_empty());
    assert_eq!(budget.stats().read_bytes, 0);
    assert_eq!(filesystem.effect_origin(), EffectOrigin::VirtualFs);
}

#[test]
fn read_limit_precedes_content_materialization() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let path = VPath::parse("data.bin").unwrap();
    let policy = CallPolicy::default();
    let mut budget = ExecutionBudget::new(ExecutionLimits {
        max_io_call_bytes: 2,
        ..ExecutionLimits::default()
    });
    assert!(matches!(
        FsGateway::new(
            &mut filesystem,
            &policy,
            &mut budget,
            EffectOrigin::MontyOsCall
        )
        .read(&path),
        Err(GatewayError::Limit(_))
    ));
    assert_eq!(budget.stats().read_bytes, 0);
    assert_eq!(filesystem.effects().len(), 1);
    assert!(matches!(
        filesystem.effects()[0].effect,
        Effect::MetadataRead { .. }
    ));
    assert!(filesystem.read_set()[&path].content.is_none());
}

#[test]
fn append_charges_old_content_and_new_payload_with_existing_dependencies() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let path = VPath::parse("data.bin").unwrap();
    let policy = CallPolicy::default();
    let mut budget = ExecutionBudget::new(ExecutionLimits::default());
    FsGateway::new(
        &mut filesystem,
        &policy,
        &mut budget,
        EffectOrigin::MontyToolCall,
    )
    .append(&path, b"!")
    .unwrap();
    assert_eq!(budget.stats().read_bytes, 3);
    assert_eq!(budget.stats().write_bytes, 1);
    assert!(filesystem.read_set()[&path].content.is_some());
    assert!(filesystem.write_set().contains_key(&path));
    assert!(
        filesystem
            .effects()
            .iter()
            .all(|event| event.origin == EffectOrigin::MontyToolCall)
    );
    assert_eq!(filesystem.read(&path).unwrap(), [0xff, 0, 0xfe, b'!']);
    assert_eq!(
        filesystem.effects().last().unwrap().origin,
        EffectOrigin::VirtualFs
    );
}

#[test]
fn append_failure_never_applies_partial_content_and_does_not_refund_work() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let path = VPath::parse("data.bin").unwrap();
    let policy = CallPolicy::default();
    let mut budget = ExecutionBudget::new(ExecutionLimits {
        max_read_bytes: 2,
        ..ExecutionLimits::default()
    });
    assert!(matches!(
        FsGateway::new(
            &mut filesystem,
            &policy,
            &mut budget,
            EffectOrigin::MontyOsCall
        )
        .append(&path, b"!"),
        Err(GatewayError::Limit(_))
    ));
    assert_eq!(budget.stats().write_bytes, 1);
    assert_eq!(budget.stats().read_bytes, 0);
    assert!(filesystem.canonical_diff().unwrap().is_empty());
    assert_eq!(filesystem.read(&path).unwrap(), [0xff, 0, 0xfe]);
}

#[test]
fn append_missing_file_records_absence_then_creates_it() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let path = VPath::parse("new.txt").unwrap();
    let policy = CallPolicy::default();
    let mut budget = ExecutionBudget::new(ExecutionLimits::default());
    FsGateway::new(
        &mut filesystem,
        &policy,
        &mut budget,
        EffectOrigin::MontyOsCall,
    )
    .append(&path, b"new")
    .unwrap();
    assert_eq!(filesystem.read(&path).unwrap(), b"new");
    assert_eq!(budget.stats().read_bytes, 0);
    assert_eq!(budget.stats().write_bytes, 3);
    assert!(filesystem.write_set().contains_key(&path));
    assert!(filesystem.effects().iter().any(|event| matches!(&event.effect, Effect::MetadataRead { path: observed, state: None } if observed == &path)));
}

#[test]
fn directory_limit_never_returns_a_successful_partial_or_hidden_listing() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let path = VPath::parse("folder").unwrap();
    let policy = CallPolicy::new(vec![
        ProtectedRule::new("folder/secret.txt", AccessSet::METADATA_READ).unwrap(),
    ]);
    let mut budget = ExecutionBudget::new(ExecutionLimits {
        max_directory_entries: 1,
        ..ExecutionLimits::default()
    });
    let result = FsGateway::new(
        &mut filesystem,
        &policy,
        &mut budget,
        EffectOrigin::MontyOsCall,
    )
    .read_dir(&path);
    assert!(matches!(
        result,
        Err(GatewayError::Limit(
            vsh_execution::ExecutionLimitExceeded::DirectoryEntries {
                limit: 1,
                attempted: 2,
            }
        ))
    ));
    assert_eq!(budget.stats().directory_entries, 0);
    assert!(filesystem.effects().is_empty());
    assert!(filesystem.canonical_diff().unwrap().is_empty());
}

#[test]
fn directory_limits_account_for_visible_overlay_entries_and_remaining_work() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let path = VPath::parse("folder").unwrap();
    let visible = VPath::parse("folder/visible.txt").unwrap();
    filesystem
        .unlink(&VPath::parse("folder/secret.txt").unwrap())
        .unwrap();
    let policy = CallPolicy::default();
    let mut budget = ExecutionBudget::new(ExecutionLimits {
        max_directory_entries: 1,
        ..ExecutionLimits::default()
    });
    {
        let mut gateway = FsGateway::new(
            &mut filesystem,
            &policy,
            &mut budget,
            EffectOrigin::MontyOsCall,
        );
        assert_eq!(gateway.read_dir(&path).unwrap(), [visible]);
        assert!(matches!(
            gateway.read_dir(&path),
            Err(GatewayError::Limit(
                vsh_execution::ExecutionLimitExceeded::DirectoryEntries {
                    limit: 1,
                    attempted: 2,
                }
            ))
        ));
    }
    assert_eq!(budget.stats().directory_entries, 1);
    let observed = filesystem
        .effects()
        .iter()
        .filter(|event| matches!(event.effect, Effect::DirectoryRead { .. }))
        .count();
    assert_eq!(observed, 1);

    filesystem
        .mkdir(&VPath::parse("empty").unwrap(), 0o755)
        .unwrap();
    let mut zero = ExecutionBudget::new(ExecutionLimits {
        max_directory_entries: 0,
        ..ExecutionLimits::default()
    });
    assert!(
        FsGateway::new(
            &mut filesystem,
            &policy,
            &mut zero,
            EffectOrigin::MontyOsCall
        )
        .read_dir(&VPath::parse("empty").unwrap())
        .unwrap()
        .is_empty()
    );
    filesystem
        .write(&VPath::parse("folder/new.txt").unwrap(), b"new")
        .unwrap();
    assert!(matches!(
        filesystem.read_dir_with_limit(&path, 1),
        Err(VfsError::DirectoryEntryLimit {
            limit: 1,
            attempted: 2,
            ..
        })
    ));
    assert_eq!(
        filesystem.read_dir_with_limit(&path, 2).unwrap(),
        [
            VPath::parse("folder/new.txt").unwrap(),
            VPath::parse("folder/visible.txt").unwrap()
        ]
    );
}

#[test]
fn empty_directory_deletion_cannot_bypass_listing_policy_or_budget() {
    let fixture = Fixture::new();
    let path = VPath::parse("folder").unwrap();
    for use_remove in [false, true] {
        let mut filesystem = fixture.filesystem();
        let policy = CallPolicy::default();
        let mut budget = ExecutionBudget::new(ExecutionLimits {
            max_directory_entries: 0,
            ..ExecutionLimits::default()
        });
        let mut gateway = FsGateway::new(
            &mut filesystem,
            &policy,
            &mut budget,
            EffectOrigin::MontyOsCall,
        );
        let result = if use_remove {
            gateway.remove(&path, false, false)
        } else {
            gateway.rmdir(&path)
        };
        assert!(matches!(result, Err(GatewayError::Limit(_))));
        assert_eq!(budget.stats().directory_entries, 0);
        assert!(filesystem.write_set().is_empty());
        assert!(
            !filesystem
                .effects()
                .iter()
                .any(|event| matches!(event.effect, Effect::DirectoryRead { .. }))
        );

        let mut filesystem = fixture.filesystem();
        let policy = CallPolicy::new(vec![
            ProtectedRule::new("folder", AccessSet::DIRECTORY_READ).unwrap(),
        ]);
        let mut budget = ExecutionBudget::new(ExecutionLimits::default());
        let mut gateway = FsGateway::new(
            &mut filesystem,
            &policy,
            &mut budget,
            EffectOrigin::MontyOsCall,
        );
        let result = if use_remove {
            gateway.remove(&path, false, false)
        } else {
            gateway.rmdir(&path)
        };
        assert!(matches!(result, Err(GatewayError::Policy(_))));
        assert!(filesystem.canonical_diff().unwrap().is_empty());
        assert!(
            !filesystem
                .effects()
                .iter()
                .any(|event| matches!(event.effect, Effect::DirectoryRead { .. }))
        );
    }
}

#[test]
fn failed_directory_listing_and_unlink_retain_absence_dependencies() {
    let fixture = Fixture::new();
    let path = VPath::parse("missing").unwrap();
    let policy = CallPolicy::default();
    for unlink in [false, true] {
        let mut filesystem = fixture.filesystem();
        let mut budget = ExecutionBudget::new(ExecutionLimits::default());
        let mut gateway = FsGateway::new(
            &mut filesystem,
            &policy,
            &mut budget,
            EffectOrigin::MontyOsCall,
        );
        let result = if unlink {
            gateway.unlink(&path)
        } else {
            gateway.read_dir(&path).map(|_| ())
        };
        assert!(
            matches!(result, Err(GatewayError::Filesystem(source)) if matches!(*source, VfsError::NotFound { .. }))
        );
        assert_eq!(filesystem.read_set()[&path].metadata, Some(None));
        assert!(filesystem.write_set().is_empty());
    }
}

#[test]
fn structurally_invalid_rename_does_not_enumerate_or_charge_children() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let policy = CallPolicy::default();
    let mut budget = ExecutionBudget::new(ExecutionLimits {
        max_directory_entries: 0,
        ..ExecutionLimits::default()
    });
    let result = FsGateway::new(
        &mut filesystem,
        &policy,
        &mut budget,
        EffectOrigin::MontyOsCall,
    )
    .rename(
        &VPath::parse("folder").unwrap(),
        &VPath::parse("folder/sub").unwrap(),
    );
    assert!(
        matches!(result, Err(GatewayError::Filesystem(source)) if matches!(*source, VfsError::InvalidRename { .. }))
    );
    assert_eq!(budget.stats().directory_entries, 0);
    assert!(filesystem.effects().is_empty());
    assert!(filesystem.write_set().is_empty());
}

#[test]
fn mutation_tree_preflight_stops_at_listing_limit_without_partial_changes() {
    let fixture = Fixture::new();
    let policy = CallPolicy::default();
    let source = VPath::parse("folder").unwrap();
    let target = VPath::parse("target").unwrap();
    for operation in ["copy", "remove", "rename"] {
        let mut filesystem = fixture.filesystem();
        let mut budget = ExecutionBudget::new(ExecutionLimits {
            max_directory_entries: 1,
            ..ExecutionLimits::default()
        });
        let mut gateway = FsGateway::new(
            &mut filesystem,
            &policy,
            &mut budget,
            EffectOrigin::MontyToolCall,
        );
        let result = match operation {
            "copy" => gateway.copy(&source, &target, true, false),
            "remove" => gateway.remove_tree(&source),
            _ => gateway.rename(&source, &target),
        };
        assert!(matches!(result, Err(GatewayError::Limit(_))), "{operation}");
        assert_eq!(budget.stats().directory_entries, 0, "{operation}");
        assert!(
            filesystem.canonical_diff().unwrap().is_empty(),
            "{operation}"
        );
        assert!(
            filesystem
                .effects()
                .iter()
                .all(|event| { !matches!(event.effect, Effect::DirectoryRead { .. }) }),
            "{operation}"
        );
    }
}

#[test]
fn hidden_listing_names_still_consume_work_and_preserve_full_listing_dependency() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let path = VPath::parse("folder").unwrap();
    let policy = CallPolicy::new(vec![
        ProtectedRule::new("folder/secret.txt", AccessSet::METADATA_READ).unwrap(),
    ]);
    let mut budget = ExecutionBudget::new(ExecutionLimits::default());
    let children = FsGateway::new(
        &mut filesystem,
        &policy,
        &mut budget,
        EffectOrigin::MontyOsCall,
    )
    .read_dir(&path)
    .unwrap();
    assert_eq!(children, vec![VPath::parse("folder/visible.txt").unwrap()]);
    assert_eq!(budget.stats().directory_entries, 2);
    assert!(filesystem.read_set()[&path].directory.is_some());
    assert!(
        filesystem
            .effects()
            .iter()
            .all(|event| event.origin == EffectOrigin::MontyOsCall)
    );
}

#[test]
fn forbidden_recursive_child_prevents_all_deletion() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let root = VPath::parse("folder").unwrap();
    let policy = CallPolicy::new(vec![
        ProtectedRule::new("folder/secret.txt", AccessSet::DELETE).unwrap(),
    ]);
    let mut budget = ExecutionBudget::new(ExecutionLimits::default());
    assert!(matches!(
        FsGateway::new(
            &mut filesystem,
            &policy,
            &mut budget,
            EffectOrigin::MontyToolCall
        )
        .remove_tree(&root),
        Err(GatewayError::Policy(_))
    ));
    assert!(filesystem.canonical_diff().unwrap().is_empty());
    assert!(filesystem.write_set().is_empty());
    assert_eq!(budget.stats().directory_entries, 2);
    assert!(
        !filesystem
            .effects()
            .iter()
            .any(|event| matches!(event.effect, Effect::Delete { .. }))
    );
}

#[test]
fn successful_recursive_deletion_covers_every_child() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let policy = CallPolicy::default();
    let mut budget = ExecutionBudget::new(ExecutionLimits::default());
    FsGateway::new(
        &mut filesystem,
        &policy,
        &mut budget,
        EffectOrigin::MontyToolCall,
    )
    .remove_tree(&VPath::parse("folder").unwrap())
    .unwrap();
    let diff = filesystem.canonical_diff().unwrap();
    assert_eq!(
        diff.entries()
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        ["folder", "folder/secret.txt", "folder/visible.txt"]
    );
    assert_eq!(budget.stats().directory_entries, 2);
    assert!(
        filesystem
            .effects()
            .iter()
            .all(|event| event.origin == EffectOrigin::MontyToolCall)
    );
}

#[test]
fn opaque_symlink_read_never_follows_host_target() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let policy = CallPolicy::default();
    let mut budget = ExecutionBudget::new(ExecutionLimits::default());
    let link = VPath::parse("link").unwrap();
    let bytes = FsGateway::new(
        &mut filesystem,
        &policy,
        &mut budget,
        EffectOrigin::MontyOsCall,
    )
    .read_link(&link)
    .unwrap();
    assert_eq!(bytes, b"/etc/passwd");
    assert_eq!(budget.stats().read_bytes, 11);
    assert!(
        matches!(FsGateway::new(&mut filesystem, &policy, &mut budget, EffectOrigin::MontyOsCall).read(&link), Err(GatewayError::Filesystem(error)) if matches!(*error, VfsError::NotFile { .. }))
    );
    assert!(filesystem.canonical_diff().unwrap().is_empty());
}

#[test]
fn subtree_rename_preflights_protected_sources_and_rebased_destinations() {
    let fixture = Fixture::new();
    let source = VPath::parse("folder").unwrap();
    let destination = VPath::parse("moved").unwrap();
    for (pattern, denied) in [
        ("folder/secret.txt", AccessSet::RENAME_SOURCE),
        ("moved/secret.txt", AccessSet::RENAME_DESTINATION),
    ] {
        let mut filesystem = fixture.filesystem();
        let policy = CallPolicy::new(vec![ProtectedRule::new(pattern, denied).unwrap()]);
        let mut budget = ExecutionBudget::new(ExecutionLimits::default());
        assert!(matches!(
            FsGateway::new(
                &mut filesystem,
                &policy,
                &mut budget,
                EffectOrigin::MontyOsCall
            )
            .rename(&source, &destination),
            Err(GatewayError::Policy(_))
        ));
        assert!(filesystem.canonical_diff().unwrap().is_empty());
        assert!(filesystem.write_set().is_empty());
        assert!(filesystem.exists(&source).unwrap());
        assert!(!filesystem.exists(&destination).unwrap());
    }
}

#[test]
fn subtree_rename_accounts_traversal_and_rejects_long_rebased_paths() {
    let fixture = Fixture::new();
    let source = VPath::parse("folder").unwrap();
    let destination = VPath::parse("moved").unwrap();
    let policy = CallPolicy::default();
    let mut filesystem = fixture.filesystem();
    let mut budget = ExecutionBudget::new(ExecutionLimits::default());
    FsGateway::new(
        &mut filesystem,
        &policy,
        &mut budget,
        EffectOrigin::MontyOsCall,
    )
    .rename(&source, &destination)
    .unwrap();
    assert_eq!(budget.stats().directory_entries, 2);
    assert!(
        filesystem
            .exists(&VPath::parse("moved/secret.txt").unwrap())
            .unwrap()
    );
    assert!(!filesystem.exists(&source).unwrap());
    let mut filesystem = fixture.filesystem();
    let mut budget = ExecutionBudget::new(ExecutionLimits {
        max_path_bytes: 18,
        ..ExecutionLimits::default()
    });
    let long_destination = VPath::parse("longer_folder").unwrap();
    assert!(matches!(
        FsGateway::new(
            &mut filesystem,
            &policy,
            &mut budget,
            EffectOrigin::MontyOsCall
        )
        .rename(&source, &long_destination),
        Err(GatewayError::Limit(_))
    ));
    assert!(filesystem.canonical_diff().unwrap().is_empty());
}

#[test]
fn metadata_absence_and_file_directory_lifecycle_use_normal_vfs_preconditions() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let policy = CallPolicy::default();
    let mut budget = ExecutionBudget::new(ExecutionLimits::default());
    let root = VPath::parse("new").unwrap();
    let file = VPath::parse("new/file").unwrap();
    let mut gateway = FsGateway::new(
        &mut filesystem,
        &policy,
        &mut budget,
        EffectOrigin::MontyOsCall,
    );
    assert!(
        matches!(gateway.metadata(&root), Err(GatewayError::Filesystem(source)) if matches!(*source, VfsError::NotFound { .. }))
    );
    gateway.mkdir(&root, 0o700).unwrap();
    gateway.write(&file, b"hello").unwrap();
    assert_eq!(
        gateway.metadata(&root).unwrap().mode(),
        if cfg!(windows) { 0o777 } else { 0o700 }
    );
    assert!(gateway.rmdir(&root).is_err());
    gateway.unlink(&file).unwrap();
    gateway.rmdir(&root).unwrap();
    assert_eq!(budget.stats().write_bytes, 5);
    assert!(filesystem.canonical_diff().unwrap().is_empty());
    assert!(filesystem.write_set().contains_key(&root));
    assert!(filesystem.write_set().contains_key(&file));
}

#[test]
fn tree_copy_preflights_source_reads_and_every_destination_before_creating() {
    let fixture = Fixture::new();
    let source = VPath::parse("folder").unwrap();
    let destination = VPath::parse("copied").unwrap();
    for (pattern, denied) in [
        ("folder/visible.txt", AccessSet::CONTENT_READ),
        ("copied/visible.txt", AccessSet::CREATE),
    ] {
        let mut filesystem = fixture.filesystem();
        let policy = CallPolicy::new(vec![ProtectedRule::new(pattern, denied).unwrap()]);
        let mut budget = ExecutionBudget::new(ExecutionLimits::default());
        assert!(matches!(
            FsGateway::new(
                &mut filesystem,
                &policy,
                &mut budget,
                EffectOrigin::MontyToolCall
            )
            .copy(&source, &destination, true, false),
            Err(GatewayError::Policy(_))
        ));
        assert!(filesystem.canonical_diff().unwrap().is_empty());
        assert!(filesystem.write_set().is_empty());
        assert_eq!(budget.stats().directory_entries, 2);
        assert_eq!(budget.stats().read_bytes, 6);
    }
}

#[test]
fn tree_copy_preserves_directory_mode_and_preflights_payload_limits() {
    let fixture = Fixture::new();
    let policy = CallPolicy::default();
    let source = VPath::parse("folder").unwrap();
    let destination = VPath::parse("copied").unwrap();
    let mut filesystem = fixture.filesystem();
    let mut budget = ExecutionBudget::new(ExecutionLimits::default());
    FsGateway::new(
        &mut filesystem,
        &policy,
        &mut budget,
        EffectOrigin::MontyToolCall,
    )
    .copy(&source, &destination, true, false)
    .unwrap();
    assert_eq!(
        filesystem
            .read(&VPath::parse("copied/visible.txt").unwrap())
            .unwrap(),
        b"hello"
    );
    assert_eq!(
        filesystem.metadata(&destination).unwrap().mode(),
        if cfg!(windows) { 0o777 } else { 0o755 }
    );
    assert_eq!(budget.stats().read_bytes, 11);
    assert_eq!(budget.stats().write_bytes, 11);
    let mut filesystem = fixture.filesystem();
    let mut budget = ExecutionBudget::new(ExecutionLimits {
        max_write_bytes: 10,
        ..ExecutionLimits::default()
    });
    assert!(matches!(
        FsGateway::new(
            &mut filesystem,
            &policy,
            &mut budget,
            EffectOrigin::MontyToolCall
        )
        .copy(&source, &destination, true, false),
        Err(GatewayError::Limit(_))
    ));
    assert!(filesystem.canonical_diff().unwrap().is_empty());
}

#[test]
fn copy_rejects_overlap_merge_and_symlinks_instead_of_successful_stubs() {
    let fixture = Fixture::new();
    let policy = CallPolicy::default();
    let mut filesystem = fixture.filesystem();
    let mut budget = ExecutionBudget::new(ExecutionLimits::default());
    let folder = VPath::parse("folder").unwrap();
    let file = VPath::parse("data.bin").unwrap();
    let link = VPath::parse("link").unwrap();
    let destination = VPath::parse("copy").unwrap();
    let mut gateway = FsGateway::new(
        &mut filesystem,
        &policy,
        &mut budget,
        EffectOrigin::MontyToolCall,
    );
    assert!(matches!(
        gateway.copy(&folder, &folder, true, false),
        Err(GatewayError::InvalidOperation { .. })
    ));
    assert!(matches!(
        gateway.copy(&folder, &VPath::parse("folder/new").unwrap(), true, false),
        Err(GatewayError::InvalidOperation { .. })
    ));
    assert!(matches!(
        gateway.copy(&folder, &destination, false, false),
        Err(GatewayError::InvalidOperation { .. })
    ));
    assert!(matches!(
        gateway.copy(&link, &destination, false, false),
        Err(GatewayError::InvalidOperation { .. })
    ));
    assert!(gateway.copy(&file, &folder, false, true).is_err());
    assert!(gateway.copy(&file, &file, false, true).is_err());
    assert!(filesystem.canonical_diff().unwrap().is_empty());
    filesystem
        .rename(&link, &VPath::parse("folder/link").unwrap())
        .unwrap();
    let before = filesystem.canonical_diff().unwrap().digest();
    assert!(matches!(
        FsGateway::new(
            &mut filesystem,
            &policy,
            &mut budget,
            EffectOrigin::MontyToolCall
        )
        .copy(&folder, &destination, true, false),
        Err(GatewayError::InvalidOperation { path: Some(_), .. })
    ));
    assert_eq!(filesystem.canonical_diff().unwrap().digest(), before);
}

#[test]
fn regular_file_copy_obeys_overwrite_and_existing_destination_mode() {
    let fixture = Fixture::new();
    let policy = CallPolicy::default();
    let mut filesystem = fixture.filesystem();
    let mut budget = ExecutionBudget::new(ExecutionLimits::default());
    let source = VPath::parse("folder/visible.txt").unwrap();
    let destination = VPath::parse("data.bin").unwrap();
    assert!(
        FsGateway::new(
            &mut filesystem,
            &policy,
            &mut budget,
            EffectOrigin::MontyToolCall
        )
        .copy(&source, &destination, false, false)
        .is_err()
    );
    assert!(filesystem.canonical_diff().unwrap().is_empty());
    FsGateway::new(
        &mut filesystem,
        &policy,
        &mut budget,
        EffectOrigin::MontyToolCall,
    )
    .copy(&source, &destination, false, true)
    .unwrap();
    assert_eq!(filesystem.read(&destination).unwrap(), b"hello");
    assert_eq!(filesystem.metadata(&destination).unwrap().mode(), 0o600);
    assert_eq!(budget.stats().read_bytes, 5);
    assert_eq!(budget.stats().write_bytes, 5);
}

#[test]
fn visible_walk_reads_observed_sizes_without_duplicate_metadata_or_raw_handles() {
    let fixture = Fixture::new();
    let policy = CallPolicy::default();
    let mut filesystem = fixture.filesystem();
    let mut budget = ExecutionBudget::new(ExecutionLimits::default());
    let mut found = Vec::new();
    FsGateway::new(
        &mut filesystem,
        &policy,
        &mut budget,
        EffectOrigin::MontyToolCall,
    )
    .walk_visible::<GatewayError>(&VPath::parse("folder").unwrap(), |mut entry| {
        let bytes = entry.read()?;
        found.push((entry.path().to_string(), bytes));
        Ok(true)
    })
    .unwrap();
    assert_eq!(
        found,
        [
            ("folder/secret.txt".to_owned(), b"secret".to_vec()),
            ("folder/visible.txt".to_owned(), b"hello".to_vec())
        ]
    );
    assert_eq!(budget.stats().read_bytes, 11);
    assert_eq!(budget.stats().directory_entries, 2);
    assert_eq!(
        filesystem
            .effects()
            .iter()
            .filter(|event| matches!(event.effect, Effect::MetadataRead { .. }))
            .count(),
        3
    );
}

#[test]
fn visible_walk_stops_before_child_enumeration_and_hides_metadata_denials() {
    let fixture = Fixture::new();
    let policy = CallPolicy::new(vec![
        ProtectedRule::new("folder/secret.txt", AccessSet::METADATA_READ).unwrap(),
    ]);
    let mut filesystem = fixture.filesystem();
    let mut budget = ExecutionBudget::new(ExecutionLimits::default());
    let mut found = Vec::new();
    FsGateway::new(
        &mut filesystem,
        &policy,
        &mut budget,
        EffectOrigin::MontyToolCall,
    )
    .walk_visible::<GatewayError>(&VPath::parse("folder").unwrap(), |entry| {
        found.push(entry.path().to_string());
        Ok(true)
    })
    .unwrap();
    assert_eq!(found, ["folder/visible.txt"]);
    let policy = CallPolicy::default();
    let mut filesystem = fixture.filesystem();
    let mut budget = ExecutionBudget::new(ExecutionLimits::default());
    FsGateway::new(
        &mut filesystem,
        &policy,
        &mut budget,
        EffectOrigin::MontyToolCall,
    )
    .walk_visible::<GatewayError>(&VPath::root(), |_| Ok(false))
    .unwrap();
    assert_eq!(budget.stats().directory_entries, 3);
    assert_eq!(
        filesystem
            .effects()
            .iter()
            .filter(|event| matches!(event.effect, Effect::DirectoryRead { .. }))
            .count(),
        1
    );
}

#[test]
fn handle_preparation_separates_read_update_write_and_append_authority() {
    let fixture = Fixture::new();
    let path = VPath::parse("data.bin").unwrap();
    let policy = CallPolicy::new(vec![
        ProtectedRule::new("data.bin", AccessSet::CONTENT_READ).unwrap(),
    ]);
    for mode in [
        OpenMode::Read,
        OpenMode::ReadUpdate,
        OpenMode::WriteUpdate,
        OpenMode::AppendUpdate,
    ] {
        let mut filesystem = fixture.filesystem();
        let mut budget = ExecutionBudget::new(ExecutionLimits::default());
        assert!(matches!(
            FsGateway::new(
                &mut filesystem,
                &policy,
                &mut budget,
                EffectOrigin::MontyOsCall
            )
            .prepare_open(&path, mode),
            Err(GatewayError::Policy(_))
        ));
        assert!(filesystem.canonical_diff().unwrap().is_empty());
        assert!(filesystem.effects().is_empty());
    }
    let mut filesystem = fixture.filesystem();
    let mut budget = ExecutionBudget::new(ExecutionLimits::default());
    FsGateway::new(
        &mut filesystem,
        &policy,
        &mut budget,
        EffectOrigin::MontyOsCall,
    )
    .prepare_open(&path, OpenMode::Append)
    .unwrap();
    assert!(filesystem.canonical_diff().unwrap().is_empty());
    FsGateway::new(
        &mut filesystem,
        &policy,
        &mut budget,
        EffectOrigin::MontyOsCall,
    )
    .prepare_open(&path, OpenMode::Write)
    .unwrap();
    assert_eq!(filesystem.read(&path).unwrap(), b"");
}

#[test]
fn handle_preparation_preserves_modes_bytes_and_deferred_content_accounting() {
    let fixture = Fixture::new();
    let policy = CallPolicy::default();
    let path = VPath::parse("data.bin").unwrap();
    for mode in [
        OpenMode::Read,
        OpenMode::ReadUpdate,
        OpenMode::Write,
        OpenMode::WriteUpdate,
        OpenMode::Append,
        OpenMode::AppendUpdate,
    ] {
        let mut filesystem = fixture.filesystem();
        let mut budget = ExecutionBudget::new(ExecutionLimits {
            max_read_bytes: 0,
            max_write_bytes: 0,
            ..ExecutionLimits::default()
        });
        FsGateway::new(
            &mut filesystem,
            &policy,
            &mut budget,
            EffectOrigin::MontyOsCall,
        )
        .prepare_open(&path, mode)
        .unwrap();
        assert_eq!(budget.stats().read_bytes, 0, "{mode:?}");
        assert_eq!(budget.stats().write_bytes, 0, "{mode:?}");
        assert_eq!(
            filesystem.metadata(&path).unwrap().mode(),
            0o600,
            "{mode:?}"
        );
        let bytes = filesystem.read(&path).unwrap();
        if matches!(mode, OpenMode::Write | OpenMode::WriteUpdate) {
            assert!(bytes.is_empty(), "{mode:?}");
        } else {
            assert_eq!(bytes, [0xff, 0, 0xfe], "{mode:?}");
            assert!(filesystem.canonical_diff().unwrap().is_empty(), "{mode:?}");
        }
    }
    for mode in [
        OpenMode::Write,
        OpenMode::WriteUpdate,
        OpenMode::Append,
        OpenMode::AppendUpdate,
    ] {
        let mut filesystem = fixture.filesystem();
        let mut budget = ExecutionBudget::new(ExecutionLimits::default());
        let path = VPath::parse("new.txt").unwrap();
        FsGateway::new(
            &mut filesystem,
            &policy,
            &mut budget,
            EffectOrigin::MontyOsCall,
        )
        .prepare_open(&path, mode)
        .unwrap();
        assert!(filesystem.read(&path).unwrap().is_empty(), "{mode:?}");
        assert!(filesystem.write_set().contains_key(&path), "{mode:?}");
        assert_eq!(
            filesystem.metadata(&path).unwrap().mode(),
            if cfg!(windows) { 0o666 } else { 0o644 },
            "{mode:?}"
        );
    }
}

#[test]
fn handle_preparation_rejects_missing_read_targets_bad_parents_and_nonfiles() {
    let fixture = Fixture::new();
    let policy = CallPolicy::default();
    for mode in [
        OpenMode::Read,
        OpenMode::ReadUpdate,
        OpenMode::Write,
        OpenMode::WriteUpdate,
        OpenMode::Append,
        OpenMode::AppendUpdate,
    ] {
        for raw in ["folder", "link", "absent/file"] {
            let mut filesystem = fixture.filesystem();
            let mut budget = ExecutionBudget::new(ExecutionLimits::default());
            let path = VPath::parse(raw).unwrap();
            let result = FsGateway::new(
                &mut filesystem,
                &policy,
                &mut budget,
                EffectOrigin::MontyOsCall,
            )
            .prepare_open(&path, mode);
            assert!(
                matches!(result, Err(GatewayError::Filesystem(_))),
                "{mode:?}: {raw}"
            );
            assert!(
                filesystem.canonical_diff().unwrap().is_empty(),
                "{mode:?}: {raw}"
            );
            assert!(filesystem.write_set().is_empty(), "{mode:?}: {raw}");
        }
    }
    for mode in [OpenMode::Read, OpenMode::ReadUpdate] {
        let mut filesystem = fixture.filesystem();
        let mut budget = ExecutionBudget::new(ExecutionLimits::default());
        let result = FsGateway::new(
            &mut filesystem,
            &policy,
            &mut budget,
            EffectOrigin::MontyOsCall,
        )
        .prepare_open(&VPath::parse("absent").unwrap(), mode);
        assert!(matches!(result, Err(GatewayError::Filesystem(source))
            if matches!(*source, VfsError::NotFound { .. })));
        assert!(filesystem.canonical_diff().unwrap().is_empty());
    }
}

#[test]
fn directory_creation_preflights_ancestors_and_handles_existing_nodes_explicitly() {
    let fixture = Fixture::new();
    let path = VPath::parse("blocked/child/leaf").unwrap();
    let policy = CallPolicy::new(vec![
        ProtectedRule::new("blocked", AccessSet::CREATE).unwrap(),
    ]);
    let mut filesystem = fixture.filesystem();
    let mut budget = ExecutionBudget::new(ExecutionLimits::default());
    assert!(matches!(
        FsGateway::new(
            &mut filesystem,
            &policy,
            &mut budget,
            EffectOrigin::MontyOsCall
        )
        .mkdir_options(&path, 0o700, true, false),
        Err(GatewayError::Policy(_))
    ));
    assert!(filesystem.canonical_diff().unwrap().is_empty());
    let policy = CallPolicy::default();
    let mut gateway = FsGateway::new(
        &mut filesystem,
        &policy,
        &mut budget,
        EffectOrigin::MontyOsCall,
    );
    gateway.mkdir_options(&path, 0o700, true, false).unwrap();
    gateway.mkdir_options(&path, 0o700, false, true).unwrap();
    assert!(gateway.mkdir_options(&path, 0o700, false, false).is_err());
    assert_eq!(
        gateway
            .metadata(&VPath::parse("blocked").unwrap())
            .unwrap()
            .mode(),
        if cfg!(windows) { 0o777 } else { 0o700 }
    );
    assert!(
        gateway
            .mkdir_options(&VPath::parse("data.bin/child").unwrap(), 0o700, true, false)
            .is_err()
    );
}
