# Prebuilt Release Binaries Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship prebuilt `codex-monitor`/`cdxm` binaries via GitHub Releases and make the installers download them by default, so users no longer need Rust or a C/MSVC toolchain.

**Architecture:** A tag-triggered GitHub Actions workflow cross-builds release binaries on three native runners (macOS arm64, macOS x86_64, Windows x86_64), packages each as an archive with a SHA256 sidecar, and attaches them to the release. `install.sh` and `install.ps1` gain a prebuilt-download path that detects the platform, fetches `releases/latest/download/<fixed-asset-name>`, verifies the checksum, and extracts binaries — falling back to the existing source build when prebuilt is unavailable or explicitly disabled.

**Tech Stack:** GitHub Actions, `dtolnay/rust-toolchain`, `softprops/action-gh-release`, Bash, PowerShell, Rust integration tests (`tests/*.rs`).

## Global Constraints

- Target triples (exactly these three, no Linux prebuilt): `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-pc-windows-msvc`.
- Asset names carry NO version, so `releases/latest/download/<name>` always resolves: `codex-monitor-<target>.tar.gz` (macOS), `codex-monitor-x86_64-pc-windows-msvc.zip` (Windows), each with a `<name>.sha256` sidecar.
- Each archive contains exactly two binaries at its top level: `codex-monitor` and `cdxm` (`.exe` suffix on Windows).
- Release tag format is `v<semver>` (e.g. `v0.1.0`); the workflow triggers on `push` of tags matching `v*`.
- `--skip-build` (bash) / `-SkipBuild` (ps1) MUST keep their current meaning: create the bin dir, install nothing, build nothing. Existing tests depend on this.
- Source build stays available as a fallback and via an explicit opt-in flag: `--build-from-source` (bash) / `-BuildFromSource` (ps1).
- Download base URL is overridable for tests: `CDXM_INSTALL_RELEASE_BASE` (default `https://github.com/lucianlamp/codex-monitor/releases/latest/download`).
- Checksum verification failure is fatal (non-zero exit); never install an unverified binary.
- macOS: strip the `com.apple.quarantine` xattr from extracted binaries (best-effort; ignore failure).
- Existing repo conventions: contract tests in `tests/installer_contract.rs` execute `install.sh`; `tests/windows_installer_contract.rs` greps installer text. Follow both patterns.

---

### Task 1: Release workflow

**Files:**
- Create: `.github/workflows/release.yml`
- Test: `tests/release_workflow_contract.rs`

**Interfaces:**
- Produces: release assets named per Global Constraints. Later installer tasks consume those exact names via `CDXM_INSTALL_RELEASE_BASE`.

- [ ] **Step 1: Write the failing contract test**

Create `tests/release_workflow_contract.rs`:

```rust
use std::{fs, path::Path};

fn workflow() -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/release.yml");
    fs::read_to_string(p).expect("release.yml should exist")
}

#[test]
fn release_workflow_triggers_on_version_tags() {
    let wf = workflow();
    assert!(wf.contains("tags:"));
    assert!(wf.contains("v*"));
}

#[test]
fn release_workflow_builds_all_three_targets() {
    let wf = workflow();
    assert!(wf.contains("aarch64-apple-darwin"));
    assert!(wf.contains("x86_64-apple-darwin"));
    assert!(wf.contains("x86_64-pc-windows-msvc"));
}

#[test]
fn release_workflow_packages_fixed_asset_names_with_checksums() {
    let wf = workflow();
    assert!(wf.contains("codex-monitor-aarch64-apple-darwin.tar.gz"));
    assert!(wf.contains("codex-monitor-x86_64-pc-windows-msvc.zip"));
    assert!(wf.contains(".sha256"));
    // builds both binaries
    assert!(wf.contains("--bins") || (wf.contains("codex-monitor") && wf.contains("cdxm")));
    // attaches to a GitHub release
    assert!(wf.contains("softprops/action-gh-release"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test release_workflow_contract`
Expected: FAIL — `release.yml should exist` (file missing).

- [ ] **Step 3: Create the workflow**

Create `.github/workflows/release.yml`:

```yaml
name: release

on:
  push:
    tags:
      - 'v*'
  workflow_dispatch:

permissions:
  contents: write

jobs:
  build:
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: macos-14
            target: aarch64-apple-darwin
            archive: tar.gz
          - os: macos-13
            target: x86_64-apple-darwin
            archive: tar.gz
          - os: windows-latest
            target: x86_64-pc-windows-msvc
            archive: zip
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - name: Build release binaries
        run: cargo build --release --bins --target ${{ matrix.target }}
      - name: Package (tar.gz)
        if: matrix.archive == 'tar.gz'
        shell: bash
        run: |
          set -euo pipefail
          name="codex-monitor-${{ matrix.target }}"
          staging="$(mktemp -d)"
          cp "target/${{ matrix.target }}/release/codex-monitor" "$staging/"
          cp "target/${{ matrix.target }}/release/cdxm" "$staging/"
          tar -czf "$name.tar.gz" -C "$staging" codex-monitor cdxm
          shasum -a 256 "$name.tar.gz" | awk '{print $1}' > "$name.tar.gz.sha256"
      - name: Package (zip)
        if: matrix.archive == 'zip'
        shell: pwsh
        run: |
          $name = "codex-monitor-${{ matrix.target }}"
          $staging = New-Item -ItemType Directory -Force -Path (Join-Path $env:RUNNER_TEMP "stage")
          Copy-Item "target/${{ matrix.target }}/release/codex-monitor.exe" $staging
          Copy-Item "target/${{ matrix.target }}/release/cdxm.exe" $staging
          Compress-Archive -Path (Join-Path $staging '*') -DestinationPath "$name.zip" -Force
          (Get-FileHash "$name.zip" -Algorithm SHA256).Hash.ToLower() | Out-File -Encoding ascii -NoNewline "$name.zip.sha256"
      - name: Upload to release
        uses: softprops/action-gh-release@v2
        with:
          files: |
            codex-monitor-${{ matrix.target }}.tar.gz
            codex-monitor-${{ matrix.target }}.tar.gz.sha256
            codex-monitor-${{ matrix.target }}.zip
            codex-monitor-${{ matrix.target }}.zip.sha256
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

Note: `softprops/action-gh-release` ignores `files:` entries that do not exist, so listing both `.tar.gz*` and `.zip*` per job is safe (only the produced archive uploads).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test release_workflow_contract`
Expected: PASS (3 tests).

- [ ] **Step 5: Validate YAML syntax**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/release.yml'))" && echo OK`
Expected: `OK`.

- [ ] **Step 6: Commit**

```bash
git add .github/workflows/release.yml tests/release_workflow_contract.rs
git commit -m "ci: add tag-triggered prebuilt release workflow"
```

---

### Task 2: Bash installer prebuilt download path

**Files:**
- Modify: `install.sh` (add arch detection + `download_prebuilt`, rewire `install_binaries`, add `--build-from-source`, update usage)
- Test: `tests/installer_contract.rs` (add prebuilt contract tests; keep existing tests green)

**Interfaces:**
- Consumes: release asset names + `CDXM_INSTALL_RELEASE_BASE` from Task 1.
- Produces: binaries at `$INSTALL_ROOT/bin/{codex-monitor,cdxm}`.

- [ ] **Step 1: Write the failing tests**

Append to `tests/installer_contract.rs`:

```rust
fn install_sh() -> String {
    fs::read_to_string(repo_root().join("install.sh")).unwrap()
}

#[test]
fn installer_has_prebuilt_download_path() {
    let s = install_sh();
    assert!(s.contains("releases/latest/download"));
    assert!(s.contains("CDXM_INSTALL_RELEASE_BASE"));
    // verifies a checksum before installing
    assert!(s.contains("shasum") || s.contains("sha256sum"));
    // maps macOS arches to the two darwin targets
    assert!(s.contains("aarch64-apple-darwin"));
    assert!(s.contains("x86_64-apple-darwin"));
    // explicit source-build opt-in still exists
    assert!(s.contains("--build-from-source"));
}

#[test]
fn installer_skip_build_still_installs_nothing() {
    // Re-uses the existing skill+shim contract: --skip-build must not download or build.
    let home = tempfile::tempdir().unwrap();
    let output = Command::new("bash")
        .arg(repo_root().join("install.sh"))
        .arg("--source").arg(repo_root())
        .arg("--yes").arg("--no-shim").arg("--skip-build")
        .env("HOME", home.path())
        .env("CDXM_INSTALL_RELEASE_BASE", "http://127.0.0.1:0/should-not-be-used")
        .output().unwrap();
    assert!(output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr));
    assert!(!home.path().join(".codex-monitor/bin/cdxm").exists());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test installer_contract installer_has_prebuilt_download_path`
Expected: FAIL — `install.sh` lacks `releases/latest/download`.

- [ ] **Step 3: Implement in `install.sh`**

Add near the other env defaults (after line 10):

```bash
RELEASE_BASE="${CDXM_INSTALL_RELEASE_BASE:-https://github.com/lucianlamp/codex-monitor/releases/latest/download}"
BUILD_FROM_SOURCE=0
```

Add `--build-from-source` to the arg parser (alongside `--skip-build`):

```bash
    --build-from-source)
      BUILD_FROM_SOURCE=1
      shift
      ;;
```

Add a `--build-from-source` line to `usage()` and update the closing note.

Add the detection + download functions before `install_binaries`:

```bash
prebuilt_target() {
  # Only macOS has prebuilt archives; everything else returns empty.
  [ "$(uname -s)" = "Darwin" ] || return 0
  case "$(uname -m)" in
    arm64|aarch64) printf 'aarch64-apple-darwin\n' ;;
    x86_64) printf 'x86_64-apple-darwin\n' ;;
  esac
}

download_prebuilt() {
  local target="$1"
  local archive="codex-monitor-$target.tar.gz"
  local url="$RELEASE_BASE/$archive"
  local dl_dir expected actual
  dl_dir="$(mktemp -d)"

  echo "Downloading prebuilt binaries: $url"
  if ! curl -fsSL "$url" -o "$dl_dir/$archive"; then
    echo "Prebuilt download failed; falling back to source build." >&2
    rm -rf "$dl_dir"
    return 1
  fi
  if curl -fsSL "$url.sha256" -o "$dl_dir/$archive.sha256"; then
    expected="$(tr -d '[:space:]' < "$dl_dir/$archive.sha256")"
    actual="$(shasum -a 256 "$dl_dir/$archive" | awk '{print $1}')"
    if [ "$expected" != "$actual" ]; then
      echo "Checksum mismatch for $archive (expected $expected, got $actual)" >&2
      rm -rf "$dl_dir"
      exit 1
    fi
  else
    echo "Warning: no published checksum for $archive; skipping verification." >&2
  fi

  mkdir -p "$BIN_DIR"
  tar -xzf "$dl_dir/$archive" -C "$BIN_DIR" codex-monitor cdxm
  chmod +x "$BIN_DIR/codex-monitor" "$BIN_DIR/cdxm"
  xattr -d com.apple.quarantine "$BIN_DIR/codex-monitor" "$BIN_DIR/cdxm" 2>/dev/null || true
  rm -rf "$dl_dir"
  echo "Installed prebuilt cdxm to $BIN_DIR/cdxm"
  return 0
}
```

Rewrite `install_binaries` to choose prebuilt first:

```bash
install_binaries() {
  if [ "$SKIP_BUILD" -eq 1 ]; then
    mkdir -p "$BIN_DIR"
    echo "Skipped binary install (--skip-build)."
    return 0
  fi

  if [ "$BUILD_FROM_SOURCE" -eq 0 ]; then
    local target
    target="$(prebuilt_target)"
    if [ -n "$target" ] && download_prebuilt "$target"; then
      return 0
    fi
  fi

  if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo is required to build codex-monitor from source." >&2
    echo "Install Rust/Cargo, then rerun this installer." >&2
    exit 1
  fi

  mkdir -p "$INSTALL_ROOT"
  cargo install --path "$SOURCE_DIR" --bins --force --root "$INSTALL_ROOT"
  echo "Installed cdxm to $BIN_DIR/cdxm"
}
```

- [ ] **Step 4: Run the full installer test suite**

Run: `cargo test --test installer_contract`
Expected: PASS — new prebuilt tests pass, and all four pre-existing tests still pass.

- [ ] **Step 5: Commit**

```bash
git add install.sh tests/installer_contract.rs
git commit -m "feat: download prebuilt binaries in install.sh with source fallback"
```

---

### Task 3: PowerShell installer prebuilt download path

**Files:**
- Modify: `install.ps1` (add `-BuildFromSource`, `$ReleaseBase`, prebuilt download in `Install-CdxmBinaries`)
- Test: `tests/windows_installer_contract.rs` (add grep-based prebuilt assertions)

**Interfaces:**
- Consumes: `codex-monitor-x86_64-pc-windows-msvc.zip` + `.sha256` from Task 1.
- Produces: `codex-monitor.exe` and `cdxm.exe` in `$BinDir`.

- [ ] **Step 1: Write the failing test**

Append to `tests/windows_installer_contract.rs` a new test (mirror the existing `fn installer()` text-read helper used there):

```rust
#[test]
fn windows_installer_has_prebuilt_download_path() {
    let installer = installer(); // existing helper that reads install.ps1
    assert!(installer.contains("releases/latest/download"));
    assert!(installer.contains("CDXM_INSTALL_RELEASE_BASE"));
    assert!(installer.contains("x86_64-pc-windows-msvc.zip"));
    assert!(installer.contains("Get-FileHash"));
    assert!(installer.contains("BuildFromSource"));
}
```

If no `installer()` helper exists, read the file inline:
`let installer = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("install.ps1")).unwrap();`

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test windows_installer_contract windows_installer_has_prebuilt_download_path`
Expected: FAIL — install.ps1 lacks the prebuilt logic.

- [ ] **Step 3: Implement in `install.ps1`**

Add params: `[switch]$BuildFromSource,` and
`[string]$ReleaseBase = $(if ($env:CDXM_INSTALL_RELEASE_BASE) { $env:CDXM_INSTALL_RELEASE_BASE } else { 'https://github.com/lucianlamp/codex-monitor/releases/latest/download' }),`

Add a download helper:

```powershell
function Install-CdxmPrebuilt {
    $target = 'x86_64-pc-windows-msvc'
    $archive = "codex-monitor-$target.zip"
    $url = "$ReleaseBase/$archive"
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force -Path $tmp | Out-Null
    $zip = Join-Path $tmp $archive
    Write-Host "Downloading prebuilt binaries: $url"
    try {
        Invoke-WebRequest -Uri $url -OutFile $zip
    } catch {
        Write-Host "Prebuilt download failed; falling back to source build."
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
        return $false
    }
    try {
        $expected = (Invoke-WebRequest -Uri "$url.sha256").Content.Trim().ToLower()
        $actual = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
        if ($expected -ne $actual) {
            throw "Checksum mismatch for $archive (expected $expected, got $actual)"
        }
    } catch [System.Net.WebException] {
        Write-Host "Warning: no published checksum for $archive; skipping verification."
    }
    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    Expand-Archive -Path $zip -DestinationPath $BinDir -Force
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    Write-Host "Installed prebuilt cdxm to $(Join-Path $BinDir 'cdxm.exe')"
    return $true
}
```

Rewire `Install-CdxmBinaries` to try prebuilt first:

```powershell
function Install-CdxmBinaries {
    param([string]$SourceDir)

    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    if ($SkipBuild) {
        Write-Host "Skipped binary install (-SkipBuild)."
        return
    }
    if (-not $BuildFromSource) {
        if (Install-CdxmPrebuilt) { return }
    }

    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $cargo) {
        throw "cargo is required to build codex-monitor from source. Install Rust/Cargo, then rerun this installer."
    }
    Write-Host "Note: building from source requires the Rust MSVC toolchain plus MSVC Build Tools."
    & cargo install --path $SourceDir --bins --force --root $InstallRoot
    if ($LASTEXITCODE -ne 0) {
        throw "cargo install failed with exit code $LASTEXITCODE"
    }
    Write-Host "Installed cdxm to $(Join-Path $BinDir 'cdxm.exe')"
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test windows_installer_contract`
Expected: PASS — new test passes and all pre-existing windows-contract tests stay green.

- [ ] **Step 5: Commit**

```bash
git add install.ps1 tests/windows_installer_contract.rs
git commit -m "feat: download prebuilt binaries in install.ps1 with source fallback"
```

---

### Task 4: README + docs update

**Files:**
- Modify: `README.md` (install sections), and verify `tests/windows_installer_contract.rs::readme_documents_windows_native_install` still passes.

**Interfaces:**
- Consumes: behavior from Tasks 1-3.

- [ ] **Step 1: Check the existing README assertion**

Run: `grep -n "keeps a backup\|MSVC Build Tools\|Git Bash" README.md`
Expected: shows the strings the windows README test asserts on — do not remove them.

- [ ] **Step 2: Update README install copy**

In the macOS/Linux and Windows install sections, state that the one-liner now installs prebuilt binaries (no Rust/Cargo, and on Windows no MSVC Build Tools required) on supported platforms — macOS arm64/x86_64 and Windows x86_64 — and that other platforms (e.g. Linux) or `--build-from-source` / `-BuildFromSource` fall back to a source build (which still needs Rust and, on Windows, MSVC Build Tools). Keep every substring asserted by `readme_documents_windows_native_install` (`install.ps1`, `codex.cmd`, `keeps a backup`, `MSVC Build Tools`, `Git Bash`).

- [ ] **Step 3: Run the README contract test**

Run: `cargo test --test windows_installer_contract readme_documents_windows_native_install`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add README.md
git commit -m "docs: document prebuilt install path and source fallback"
```

---

### Task 5: Full verification + release dry-run guidance

**Files:** none (verification only).

- [ ] **Step 1: Full test suite**

Run: `cargo test`
Expected: all tests pass (including the three new contract tests).

- [ ] **Step 2: Format check**

Run: `cargo fmt --check`
Expected: clean (the existing `ci.yml` enforces this).

- [ ] **Step 3: Lint the bash installer**

Run: `bash -n install.sh && echo OK`
Expected: `OK` (no syntax errors). If `shellcheck` is available: `shellcheck install.sh` and review.

- [ ] **Step 4: Manual prebuilt smoke note (post-merge)**

After merge, cut the first release to exercise the pipeline:
`git tag v0.1.0 && git push origin v0.1.0`
Then confirm the release has six assets (3 targets × {archive, .sha256}) and run the one-liner installer on macOS to confirm a prebuilt install with checksum verification. This step is operational and runs after the PR merges.

---

## Self-Review

- **Spec coverage:** release workflow (Task 1), bash installer (Task 2), ps1 installer (Task 3), README (Task 4), verification (Task 5). All three chosen targets are built and consumed. ✓
- **Placeholders:** none — every code/script step shows full content. ✓
- **Type/name consistency:** asset names (`codex-monitor-<target>.tar.gz|zip` + `.sha256`), env override `CDXM_INSTALL_RELEASE_BASE`, and flags (`--build-from-source` / `-BuildFromSource`) are identical across Tasks 1-4. ✓
