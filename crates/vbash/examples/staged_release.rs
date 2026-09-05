//! Run the shared Monty release recipe against an owned disposable fixture.

use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use vsh::{PrincipalId, ReceiptDetail, RunRequest, Runtime, RuntimeConfig, RuntimeDecision};

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Result<Self, Box<dyn Error>> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path =
            std::env::temp_dir().join(format!("vsh-cookbook-{}-{nonce}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        // Only the unique directory successfully created by this guard is owned.
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let workspace = Workspace::new()?;
    fs::create_dir(workspace.0.join("templates"))?;
    fs::write(
        workspace.0.join("templates/service.toml"),
        b"channel = \"dev\"\n",
    )?;
    let runtime = Runtime::open(RuntimeConfig::new(&workspace.0))?;
    let code = include_str!("staged_release.monty");
    let preview = runtime.preview(RunRequest::new(code).with_detail(ReceiptDetail::Full))?;
    assert!(matches!(
        preview.decision,
        RuntimeDecision::PendingApproval(_)
    ));
    assert_eq!(preview.changed_paths, 3);
    assert!(!workspace.0.join("release").exists());
    assert_eq!(
        preview
            .changes
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        ["release", "release/README.txt", "release/app.toml"]
    );
    let now = u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
    // A rename is a semantic risk even if it only rearranges generated files.
    // Production code must authenticate a reviewer before this trusted call.
    runtime.approve(
        preview.transaction,
        PrincipalId::digest_label("fixture-reviewer"),
        now,
        now + 30_000,
    )?;
    let committed = runtime.commit(preview.transaction, now)?;
    assert_eq!(committed.transaction, preview.transaction);
    assert!(committed.commit.is_some());
    assert_eq!(
        fs::read_to_string(workspace.0.join("release/app.toml"))?,
        "channel = \"stable\"\n"
    );
    assert_eq!(
        fs::read_to_string(workspace.0.join("release/README.txt"))?,
        "channel=stable\n"
    );
    assert!(!workspace.0.join("release/service.toml").exists());
    println!(
        "Committed {} reviewed paths: {}",
        committed.changed_paths, committed.transaction
    );
    Ok(())
}
