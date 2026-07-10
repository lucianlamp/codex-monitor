# macOS Update and Single-Binary Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add checksum-verified native macOS self-update, converge old macOS installs on one native executable, and immediately migrate owned LaunchAgents to the canonical `cdxm` launcher.

**Architecture:** Extend the existing manifest/apply transaction with an explicit release platform and a strict tar.gz reader. Add a focused macOS finalizer that atomically writes the launcher, transactionally rewrites owned plist program paths, reloads exact previously loaded labels, and removes fixed legacy binaries only after verification.

**Tech Stack:** Rust 2021, Tokio, reqwest/rustls, sha2, flate2 `GzDecoder`, tar 0.4, launchctl, POSIX shell, Cargo integration tests.

---

## File Map

- Modify `Cargo.toml` and `Cargo.lock`: add direct `flate2` and `tar` dependencies.
- Modify `src/update/model.rs`: explicit release platform and platform-aware paths/names.
- Modify `src/update/archive.rs`: strict ZIP and tar.gz readers plus shared bounded download.
- Create `src/update/macos.rs`: macOS paths, launcher, plist migration, exact-label reload, rollback, legacy cleanup.
- Modify `src/update.rs`: macOS updater/finalizer orchestration and result reporting.
- Modify `src/update/apply.rs`: use manifest platform when preparing and verifying files.
- Modify `src/launchd.rs`: expose the small XML helpers needed by the migrator.
- Modify `src/cli.rs`: hidden macOS finalizer command.
- Modify `install.sh`: invoke installed finalizer after a real Darwin install.
- Modify `tests/installer_contract.rs`, `tests/cli_contract.rs`, and `tests/release_workflow_contract.rs`: public contract coverage.
- Modify `README.md`, `skills/codex-monitor/SKILL.md`, and `skills/codex-monitor/references/codex-monitor-operations.md`: operator documentation.

### Task 1: Make the update manifest platform-aware

**Files:**
- Modify: `src/update/model.rs`
- Modify: `src/update/apply.rs`
- Modify: `src/update.rs`

- [ ] **Step 1: Write failing model tests**

Add tests that require explicit platform-specific asset and executable names:

```rust
#[test]
fn release_platform_maps_assets_and_destinations() {
    let root = Path::new("/tmp/codex-monitor");
    assert_eq!(
        ReleasePlatform::MacArm64.archive_name(),
        "codex-monitor-aarch64-apple-darwin.tar.gz"
    );
    assert_eq!(
        ManagedFile::CodexMonitor.destination(root, ReleasePlatform::MacArm64),
        root.join("bin/codex-monitor")
    );
    assert_eq!(
        ManagedFile::CodexMonitor.destination(root, ReleasePlatform::WindowsX64),
        root.join("bin/codex-monitor.exe")
    );
}

#[test]
fn manifest_rejects_a_platform_mismatch() {
    let mut manifest = valid_manifest(ReleasePlatform::MacArm64);
    assert!(manifest.validate_for(ReleasePlatform::WindowsX64).is_err());
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run: `cargo test update::model::tests::release_platform_maps_assets_and_destinations`

Expected: compile failure because `ReleasePlatform` and platform parameters do not exist.

- [ ] **Step 3: Add the platform model and thread it through apply**

Implement these core shapes:

```rust
#[derive(Debug, Copy, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReleasePlatform {
    WindowsX64,
    MacArm64,
    MacX64,
}

impl ReleasePlatform {
    pub fn current() -> anyhow::Result<Self>;
    pub fn archive_name(self) -> &'static str;
    pub fn executable_name(self) -> &'static str;
}

pub struct UpdateManifest {
    pub version: u32,
    pub platform: ReleasePlatform,
    pub install_root: PathBuf,
    pub staging_root: PathBuf,
    pub files: Vec<StagedFile>,
}
```

Bump `MANIFEST_VERSION`, make `ManagedFile::{destination,staged_name}` accept a
`ReleasePlatform`, and make `apply_manifest` use `manifest.platform` for every
prepared, backup, destination, and verification path. Preserve the invariant of
exactly one managed native file.

- [ ] **Step 4: Run model/apply tests and verify GREEN**

Run: `cargo test update::model`

Run: `cargo test update::apply`

Expected: all selected tests pass with zero failures.

- [ ] **Step 5: Commit**

```bash
git add src/update/model.rs src/update/apply.rs src/update.rs
git commit -m "refactor: make update manifests platform aware"
```

### Task 2: Download and strictly extract macOS release tarballs

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `src/update/archive.rs`

- [ ] **Step 1: Add failing tar.gz tests**

Use `flate2::write::GzEncoder` and `tar::Builder` to construct fixtures. Require
one top-level regular file and reject nested paths, symlinks, duplicates, and
extra entries:

```rust
#[test]
fn release_targz_extracts_one_native_binary() {
    let bytes = targz_bytes(&[("codex-monitor", EntryKind::Regular(b"monitor"))]);
    let destination = TempDir::new().unwrap();
    let staged = extract_release_targz(
        &bytes,
        destination.path(),
        ReleasePlatform::MacArm64,
    ).unwrap();
    assert_eq!(staged.len(), 1);
    assert_eq!(fs::read(destination.path().join("codex-monitor")).unwrap(), b"monitor");
}

#[test]
fn release_targz_rejects_unsafe_shapes() {
    for bytes in invalid_targz_cases() {
        assert!(extract_release_targz(
            &bytes,
            TempDir::new().unwrap().path(),
            ReleasePlatform::MacArm64,
        ).is_err());
    }
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run: `cargo test update::archive::tests::release_targz`

Expected: compile failure because the tar.gz extractor is absent.

- [ ] **Step 3: Add dependencies and the strict reader**

Add direct dependencies `flate2 = "1.1"` and `tar = "0.4.46"`. Implement:

```rust
pub fn extract_release_targz(
    bytes: &[u8],
    destination: &Path,
    platform: ReleasePlatform,
) -> anyhow::Result<Vec<StagedFile>> {
    let decoder = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(decoder);
    // Iterate sequentially. Accept one regular entry whose path and raw path
    // equal platform.executable_name(); copy it into a bounded file, chmod
    // 0755 on Unix, hash it, and reject every other entry or type.
}
```

Generalize `download_latest_release(base, destination, platform)` so it fetches
`platform.archive_name()` and its `.sha256`, applies the existing 128 MiB
compressed limit, verifies the digest before parsing, then dispatches to ZIP or
tar.gz extraction.

- [ ] **Step 4: Run archive tests and verify GREEN**

Run: `cargo test update::archive`

Expected: checksum, ZIP, tar.gz, and HTTP fixture tests all pass.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/update/archive.rs
git commit -m "feat: verify and extract macOS release archives"
```

### Task 3: Add transactional macOS installation finalization

**Files:**
- Create: `src/update/macos.rs`
- Modify: `src/update.rs`
- Modify: `src/launchd.rs`

- [ ] **Step 1: Write failing launcher and plist migration tests**

Cover the canonical launcher, owned path filter, argument preservation, and
rollback through an injected launchctl adapter:

```rust
#[test]
fn launcher_executes_the_single_native_binary() {
    let temp = TempDir::new().unwrap();
    write_cdxm_launcher(temp.path()).unwrap();
    let text = fs::read_to_string(temp.path().join("bin/cdxm")).unwrap();
    assert!(text.contains("exec \"$SCRIPT_DIR/codex-monitor\" \"$@\""));
}

#[test]
fn plist_migration_changes_only_owned_first_argument() {
    let migrated = migrate_plist_program(
        &fixture_plist("/Users/me/.cargo/bin/cdxm"),
        Path::new("/Users/me/.codex-monitor/bin/cdxm"),
        &owned_paths(Path::new("/Users/me")),
    ).unwrap().unwrap();
    assert_eq!(parse_program_arguments_from_plist(&migrated)[1..], fixture_args()[1..]);
}

#[test]
fn migration_failure_restores_changed_plists_and_loaded_services() {
    let fake = FakeLaunchctl::fail_bootstrap_for("com.local.codex-monitor.agmsg.dev.two");
    assert!(finalize_with_launchctl(&fixture_paths(), &fake).is_err());
    assert_eq!(read_fixture_plists(), original_fixture_plists());
    assert_eq!(fake.restored_loaded_labels(), vec!["dev.one"]);
}
```

- [ ] **Step 2: Run focused tests and verify RED**

Run: `cargo test update::macos`

Expected: compile failure because the macOS finalizer module is absent.

- [ ] **Step 3: Implement the focused macOS module**

Create these boundaries:

```rust
pub struct MacInstallPaths {
    pub root: PathBuf,
    pub binary: PathBuf,
    pub launcher: PathBuf,
    pub update_result: PathBuf,
}

trait Launchctl {
    fn is_loaded(&self, label: &str) -> anyhow::Result<bool>;
    fn reload(&self, label: &str, plist: &Path) -> anyhow::Result<()>;
    fn active_arguments(&self, label: &str) -> anyhow::Result<Vec<String>>;
}

pub fn finalize_install() -> anyhow::Result<MacFinalizeSummary>;
pub fn write_cdxm_launcher(root: &Path) -> anyhow::Result<()>;
```

Use sibling temporary files plus rename for launcher/plist publication. Inventory
only `~/Library/LaunchAgents/com.local.codex-monitor.agmsg.*.plist`. Select only
fixed owned first arguments, snapshot bytes and loaded state, rewrite/reload
exact labels, verify canonical active arguments, and roll back already changed
plists on failure. Remove `~/.cargo/bin/{cdxm,codex-monitor}` only after the
whole migration succeeds.

- [ ] **Step 4: Run finalizer and launchd tests and verify GREEN**

Run: `cargo test update::macos`

Run: `cargo test launchd::tests`

Expected: all selected tests pass with zero failures.

- [ ] **Step 5: Commit**

```bash
git add src/update/macos.rs src/update.rs src/launchd.rs
git commit -m "feat: migrate macOS installs and LaunchAgents"
```

### Task 4: Wire the macOS updater and installer entry points

**Files:**
- Modify: `src/update.rs`
- Modify: `src/cli.rs`
- Modify: `install.sh`
- Modify: `tests/cli_contract.rs`
- Modify: `tests/installer_contract.rs`

- [ ] **Step 1: Write failing command and installer contracts**

Require a hidden finalizer and a Darwin-only installer call after real binary
installation:

```rust
#[test]
fn internal_macos_finalizer_is_hidden() {
    let help = command_output(["--help"]);
    assert!(!help.contains("__finalize-macos-install"));
}

#[test]
fn unix_installer_finalizes_real_macos_installs() {
    let script = installer_text();
    assert!(script.contains("__finalize-macos-install"));
    assert!(script.contains("Darwin"));
    assert!(script.contains("SKIP_BUILD"));
}
```

- [ ] **Step 2: Run focused contracts and verify RED**

Run: `cargo test --test cli_contract internal_macos_finalizer_is_hidden`

Run: `cargo test --test installer_contract unix_installer_finalizes_real_macos_installs`

Expected: installer contract fails because no finalizer call exists.

- [ ] **Step 3: Implement macOS update orchestration**

Add the hidden command and platform dispatch:

```rust
#[command(name = "__finalize-macos-install", hide = true)]
FinalizeMacosInstall,

#[cfg(target_os = "macos")]
pub async fn run_update() -> Result<i32> {
    run_update_macos().await
}
```

`run_update_macos` creates staging under `~/.codex-monitor`, downloads the
current platform tarball, builds/validates the manifest, applies it in-process,
runs `macos::finalize_install`, writes `last-update.json`, and cleans staging.
`report_previous_failure` consumes macOS results too. On Darwin, `install.sh`
invokes the installed hidden finalizer after a successful non-skip binary
installation. Other Unix platforms retain the current unsupported update error.

- [ ] **Step 4: Run update, CLI, and installer tests and verify GREEN**

Run: `cargo test update::`

Run: `cargo test --test cli_contract`

Run: `cargo test --test installer_contract`

Expected: all selected unit and contract suites pass.

- [ ] **Step 5: Commit**

```bash
git add src/update.rs src/cli.rs install.sh tests/cli_contract.rs tests/installer_contract.rs
git commit -m "feat: enable macOS self-update"
```

### Task 5: Document and lock the operator contract

**Files:**
- Modify: `README.md`
- Modify: `skills/codex-monitor/SKILL.md`
- Modify: `skills/codex-monitor/references/codex-monitor-operations.md`
- Modify: `tests/release_workflow_contract.rs`
- Modify: `tests/installer_contract.rs`

- [ ] **Step 1: Add failing documentation contracts**

Require docs to state arm64/Intel update support, one native binary, exact
LaunchAgent migration, and no process-name killing:

```rust
#[test]
fn docs_define_macos_update_and_single_binary_migration() {
    let readme = readme();
    assert!(readme.contains("macOS arm64 and Intel"));
    assert!(readme.contains("codex-monitor update"));
    assert!(readme.contains("LaunchAgent"));
    assert!(readme.contains("one native executable"));
}
```

- [ ] **Step 2: Run the documentation contract and verify RED**

Run: `cargo test --test installer_contract docs_define_macos_update_and_single_binary_migration`

Expected: failure because the completed migration guarantee is not documented.

- [ ] **Step 3: Update README, skill, and operations reference**

Document the asset mapping, checksum requirement, atomic update, canonical
paths, exact-label reload behavior, rollback boundary, legacy cleanup, and SSH
acceptance commands. Keep Windows behavior unchanged and Linux update explicitly
unsupported.

- [ ] **Step 4: Run contract tests and verify GREEN**

Run: `cargo test --test installer_contract --test release_workflow_contract`

Expected: both suites pass with zero failures.

- [ ] **Step 5: Commit**

```bash
git add README.md skills/codex-monitor/SKILL.md skills/codex-monitor/references/codex-monitor-operations.md tests/installer_contract.rs tests/release_workflow_contract.rs
git commit -m "docs: document macOS updater migration"
```

### Task 6: Verify locally and migrate the live Mac

**Files:**
- Verify all modified files.
- Temporary remote files only under a task-owned `/tmp/codex-monitor-macos-update-*` directory.

- [ ] **Step 1: Run the complete local quality gate**

Run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
bash -n install.sh
git diff --check
```

Expected: every command exits 0; all Rust suites report zero failures.

- [ ] **Step 2: Transfer the committed branch and build natively**

Create a Git bundle for `codex/macos-update`, copy it with `scp`, clone it into
a unique task-owned `/tmp` directory, then run:

```bash
cargo build --release --bin codex-monitor
```

Expected: an arm64 Mach-O executable at `target/release/codex-monitor`.

- [ ] **Step 3: Exercise the updater against a task-owned release server**

Package the new executable as
`codex-monitor-aarch64-apple-darwin.tar.gz`, write its SHA-256 companion, start
`python3 -m http.server` on loopback, and record the exact PID. Run:

```bash
CDXM_INSTALL_RELEASE_BASE=http://127.0.0.1:<port> \
  target/release/codex-monitor update
```

Expected: checksum verification, binary transaction, launcher finalization,
and LaunchAgent migration complete successfully.

- [ ] **Step 4: Verify the live single-binary and LaunchAgent state**

Verify:

```bash
test -x ~/.codex-monitor/bin/codex-monitor
test -f ~/.codex-monitor/bin/cdxm
file ~/.codex-monitor/bin/codex-monitor
file ~/.codex-monitor/bin/cdxm
test ! -e ~/.cargo/bin/codex-monitor
test ! -e ~/.cargo/bin/cdxm
```

For every exact `com.local.codex-monitor.agmsg.*` plist, verify the desired and
active first argument is `~/.codex-monitor/bin/cdxm`, `launchctl print` succeeds,
and all remaining arguments are unchanged from the pre-migration snapshot.

- [ ] **Step 5: Clean only task-owned remote resources**

Confirm the recorded server PID command line contains the task-owned temporary
directory, terminate that exact PID, and remove the exact task-owned `/tmp`
directory and local bundle. Do not stop any migrated LaunchAgent.

- [ ] **Step 6: Commit any acceptance-only documentation correction**

If the live run revealed a documentation mismatch, patch only that mismatch,
rerun the complete quality gate, and commit it as:

```bash
git commit -am "docs: align macOS acceptance instructions"
```

If no correction is needed, leave the worktree clean.
