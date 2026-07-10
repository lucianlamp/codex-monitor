use super::model::{sha256_bytes, sha256_file, ManagedFile, ReleasePlatform, StagedFile};
use anyhow::{bail, Context};
use flate2::read::GzDecoder;
use std::{
    collections::BTreeSet,
    fs::File,
    io::{Cursor, Read, Write},
    path::Path,
};
use tar::Archive;
use zip::ZipArchive;

const MAX_ARCHIVE_BYTES: usize = 128 * 1024 * 1024;
const MAX_EXTRACTED_FILE_BYTES: usize = 128 * 1024 * 1024;

pub fn parse_checksum(text: &str) -> anyhow::Result<String> {
    let checksum = text.trim();
    if checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("release checksum must contain exactly one SHA-256 digest");
    }
    Ok(checksum.to_ascii_lowercase())
}

pub fn verify_sha256(bytes: &[u8], expected: &str) -> anyhow::Result<()> {
    let actual = sha256_bytes(bytes);
    if actual != expected.to_ascii_lowercase() {
        bail!("release checksum mismatch (expected {expected}, got {actual})");
    }
    Ok(())
}

pub fn extract_release_zip(bytes: &[u8], destination: &Path) -> anyhow::Result<Vec<StagedFile>> {
    let platform = ReleasePlatform::WindowsX64;
    let declared_entries = declared_zip_entry_count(bytes)?;
    if declared_entries != ManagedFile::RELEASE.len() {
        bail!(
            "release ZIP must contain exactly {} entries, found {declared_entries}",
            ManagedFile::RELEASE.len()
        );
    }
    std::fs::create_dir_all(destination).with_context(|| {
        format!(
            "failed to create release staging directory {}",
            destination.display()
        )
    })?;
    let mut archive = ZipArchive::new(Cursor::new(bytes)).context("release asset is not a ZIP")?;
    let mut seen = BTreeSet::new();
    let mut staged = Vec::new();

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .with_context(|| format!("failed to read release ZIP entry {index}"))?;
        let name = entry.name().to_owned();
        if entry.is_dir()
            || name.contains('/')
            || name.contains('\\')
            || Path::new(&name)
                .file_name()
                .and_then(|value| value.to_str())
                != Some(name.as_str())
        {
            bail!("release ZIP contains a non-top-level file: {name}");
        }
        let id = ManagedFile::from_release_name(&name, platform)
            .with_context(|| format!("release ZIP contains an unexpected file: {name}"))?;
        if !seen.insert(id) {
            bail!("release ZIP contains a duplicate file: {name}");
        }

        let output_path = destination.join(id.staged_name(platform));
        let mut output = File::create(&output_path)
            .with_context(|| format!("failed to create staged file {}", output_path.display()))?;
        std::io::copy(&mut entry, &mut output)
            .with_context(|| format!("failed to extract release file {name}"))?;
        output
            .flush()
            .with_context(|| format!("failed to flush staged file {}", output_path.display()))?;
        staged.push(StagedFile {
            id,
            sha256: Some(sha256_file(&output_path)?),
        });
    }

    for required in ManagedFile::RELEASE {
        if !seen.contains(&required) {
            bail!(
                "release ZIP is missing required file: {}",
                required.staged_name(platform)
            );
        }
    }
    Ok(staged)
}

pub fn extract_release_targz(
    bytes: &[u8],
    destination: &Path,
    platform: ReleasePlatform,
) -> anyhow::Result<Vec<StagedFile>> {
    if platform == ReleasePlatform::WindowsX64 {
        bail!("Windows releases must use the ZIP archive format");
    }
    let expected_name = ManagedFile::CodexMonitor.staged_name(platform);
    let decoder = GzDecoder::new(bytes);
    let mut archive = Archive::new(decoder);
    let mut payload = None;

    for entry in archive
        .entries()
        .context("release asset is not a readable tar.gz archive")?
    {
        let entry = entry.context("failed to read release tar entry")?;
        if payload.is_some() {
            bail!("release tar.gz must contain exactly one entry");
        }
        let path = entry.path().context("release tar entry path is invalid")?;
        if path.as_ref() != Path::new(expected_name)
            || entry.path_bytes().as_ref() != expected_name.as_bytes()
        {
            bail!(
                "release tar.gz contains an unexpected or nested entry: {}",
                path.display()
            );
        }
        if !entry.header().entry_type().is_file() {
            bail!("release tar.gz entry is not a regular file: {expected_name}");
        }
        if entry.size() > MAX_EXTRACTED_FILE_BYTES as u64 {
            bail!("release executable exceeds the 128 MiB size limit");
        }
        let mut entry = entry.take(MAX_EXTRACTED_FILE_BYTES as u64 + 1);
        let mut extracted = Vec::new();
        entry
            .read_to_end(&mut extracted)
            .context("failed to read release executable")?;
        if extracted.len() > MAX_EXTRACTED_FILE_BYTES {
            bail!("release executable exceeds the 128 MiB size limit");
        }
        payload = Some(extracted);
    }

    let payload = payload.context("release tar.gz is missing codex-monitor")?;
    std::fs::create_dir_all(destination).with_context(|| {
        format!(
            "failed to create release staging directory {}",
            destination.display()
        )
    })?;
    let output_path = destination.join(expected_name);
    let mut output = File::create(&output_path)
        .with_context(|| format!("failed to create staged file {}", output_path.display()))?;
    output
        .write_all(&payload)
        .with_context(|| format!("failed to write staged file {}", output_path.display()))?;
    output
        .flush()
        .with_context(|| format!("failed to flush staged file {}", output_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&output_path, std::fs::Permissions::from_mode(0o755))
            .with_context(|| {
                format!(
                    "failed to mark staged executable as runnable {}",
                    output_path.display()
                )
            })?;
    }
    Ok(vec![StagedFile {
        id: ManagedFile::CodexMonitor,
        sha256: Some(sha256_file(&output_path)?),
    }])
}

fn declared_zip_entry_count(bytes: &[u8]) -> anyhow::Result<usize> {
    const EOCD_SIGNATURE: &[u8; 4] = b"PK\x05\x06";
    const EOCD_SIZE: usize = 22;
    if bytes.len() < EOCD_SIZE {
        bail!("release ZIP has no end-of-central-directory record");
    }
    let search_start = bytes.len().saturating_sub(EOCD_SIZE + u16::MAX as usize);
    let offset = (search_start..=bytes.len() - EOCD_SIZE)
        .rev()
        .find(|offset| &bytes[*offset..*offset + 4] == EOCD_SIGNATURE)
        .context("release ZIP has no end-of-central-directory record")?;
    let read_u16 =
        |start: usize| u16::from_le_bytes([bytes[offset + start], bytes[offset + start + 1]]);
    let disk = read_u16(4);
    let central_disk = read_u16(6);
    let entries_on_disk = read_u16(8);
    let entries = read_u16(10);
    let comment_length = read_u16(20) as usize;
    if disk != 0 || central_disk != 0 || entries_on_disk != entries {
        bail!("multi-disk release ZIP files are not supported");
    }
    if entries == u16::MAX {
        bail!("ZIP64 release archives are not supported");
    }
    if offset + EOCD_SIZE + comment_length != bytes.len() {
        bail!("release ZIP end record has an invalid comment length");
    }
    Ok(entries as usize)
}

pub async fn download_latest_release(
    release_base: &str,
    destination: &Path,
    platform: ReleasePlatform,
) -> anyhow::Result<Vec<StagedFile>> {
    let base = release_base.trim_end_matches('/');
    let archive_url = format!("{base}/{}", platform.archive_name());
    let checksum_url = format!("{archive_url}.sha256");
    let client = reqwest::Client::builder()
        .build()
        .context("failed to create update HTTP client")?;

    let checksum = client
        .get(&checksum_url)
        .send()
        .await
        .with_context(|| format!("failed to download release checksum {checksum_url}"))?
        .error_for_status()
        .with_context(|| format!("release checksum request failed: {checksum_url}"))?
        .text()
        .await
        .context("failed to read release checksum")?;
    let checksum = parse_checksum(&checksum)?;

    let response = client
        .get(&archive_url)
        .send()
        .await
        .with_context(|| format!("failed to download release archive {archive_url}"))?
        .error_for_status()
        .with_context(|| format!("release archive request failed: {archive_url}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ARCHIVE_BYTES as u64)
    {
        bail!("release archive exceeds the 128 MiB size limit");
    }
    let archive = response
        .bytes()
        .await
        .context("failed to read release archive")?;
    if archive.len() > MAX_ARCHIVE_BYTES {
        bail!("release archive exceeds the 128 MiB size limit");
    }
    verify_sha256(&archive, &checksum)?;
    if platform == ReleasePlatform::WindowsX64 {
        extract_release_zip(&archive, destination)
    } else {
        extract_release_targz(&archive, destination, platform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression};
    use std::io::{Cursor, Read, Write};
    use tar::{Builder, EntryType, Header};
    use tempfile::TempDir;
    use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

    fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, bytes) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn valid_entries() -> Vec<(&'static str, &'static [u8])> {
        vec![("codex-monitor.exe", b"monitor")]
    }

    #[derive(Clone, Copy)]
    enum TarTestEntry<'a> {
        Regular(&'a [u8]),
        Symlink(&'a str),
    }

    fn targz_bytes(entries: &[(&str, TarTestEntry<'_>)]) -> Vec<u8> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = Builder::new(encoder);
        for (path, entry) in entries {
            let mut header = Header::new_gnu();
            header.set_mode(0o755);
            match entry {
                TarTestEntry::Regular(bytes) => {
                    header.set_entry_type(EntryType::Regular);
                    header.set_size(bytes.len() as u64);
                    header.set_cksum();
                    builder
                        .append_data(&mut header, path, Cursor::new(*bytes))
                        .unwrap();
                }
                TarTestEntry::Symlink(target) => {
                    header.set_entry_type(EntryType::Symlink);
                    header.set_size(0);
                    header.set_link_name(target).unwrap();
                    header.set_cksum();
                    builder
                        .append_data(&mut header, path, std::io::empty())
                        .unwrap();
                }
            }
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn release_targz_extracts_one_native_binary() {
        let bytes = targz_bytes(&[("codex-monitor", TarTestEntry::Regular(b"monitor"))]);
        let destination = TempDir::new().unwrap();
        let staged =
            extract_release_targz(&bytes, destination.path(), ReleasePlatform::MacArm64).unwrap();
        assert_eq!(staged.len(), 1);
        assert_eq!(
            std::fs::read(destination.path().join("codex-monitor")).unwrap(),
            b"monitor"
        );
    }

    #[test]
    fn release_targz_rejects_unsafe_shapes() {
        let cases = [
            targz_bytes(&[("nested/codex-monitor", TarTestEntry::Regular(b"monitor"))]),
            targz_bytes(&[("codex-monitor", TarTestEntry::Symlink("elsewhere"))]),
            targz_bytes(&[("unexpected", TarTestEntry::Regular(b"monitor"))]),
            targz_bytes(&[
                ("codex-monitor", TarTestEntry::Regular(b"one")),
                ("codex-monitor", TarTestEntry::Regular(b"two")),
            ]),
        ];

        for (index, bytes) in cases.into_iter().enumerate() {
            let destination = TempDir::new().unwrap();
            assert!(
                extract_release_targz(&bytes, destination.path(), ReleasePlatform::MacArm64,)
                    .is_err(),
                "unsafe tar case {index} was accepted"
            );
        }
    }

    fn duplicate_name_zip() -> Vec<u8> {
        let mut archive =
            zip_bytes(&[("codex-monitor.exe", b"one"), ("codex-monitor.tmp", b"two")]);
        let old = b"codex-monitor.tmp";
        let new = b"codex-monitor.exe";
        for offset in 0..=archive.len() - old.len() {
            if &archive[offset..offset + old.len()] == old {
                archive[offset..offset + new.len()].copy_from_slice(new);
            }
        }
        assert_eq!(
            archive
                .windows(old.len())
                .filter(|bytes| *bytes == old)
                .count(),
            0
        );
        assert_eq!(declared_zip_entry_count(&archive).unwrap(), 2);
        archive
    }

    fn spawn_release_server(
        archive: Vec<u8>,
    ) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
        let checksum = sha256_bytes(&archive);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 2048];
                let read = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..read]);
                let body = if request
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .contains(".sha256")
                {
                    checksum.as_bytes()
                } else {
                    archive.as_slice()
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(body).unwrap();
            }
        });
        (address, server)
    }

    #[test]
    fn checksum_requires_exactly_one_hex_digest() {
        let digest = "a".repeat(64);
        assert_eq!(parse_checksum(&digest).unwrap(), digest);
        assert_eq!(parse_checksum(&"A".repeat(64)).unwrap(), digest);
        assert!(parse_checksum("missing").is_err());
        assert!(parse_checksum(&("a".repeat(64) + " extra")).is_err());
        assert!(parse_checksum(&("a".repeat(64) + "\n" + &"b".repeat(64))).is_err());
    }

    #[test]
    fn checksum_verification_rejects_mismatch() {
        let expected = sha256_bytes(b"archive");
        assert!(verify_sha256(b"archive", &expected).is_ok());
        assert!(verify_sha256(b"different", &expected).is_err());
    }

    #[test]
    fn release_zip_extracts_exact_binary_set() {
        let destination = TempDir::new().unwrap();
        let staged = extract_release_zip(&zip_bytes(&valid_entries()), destination.path()).unwrap();
        assert_eq!(staged.len(), 1);
        assert_eq!(
            std::fs::read(destination.path().join("codex-monitor.exe")).unwrap(),
            b"monitor"
        );
    }

    #[test]
    fn release_zip_rejects_missing_duplicate_nested_and_unexpected_members() {
        let cases = [
            zip_bytes(&[]),
            duplicate_name_zip(),
            zip_bytes(&[("nested/codex-monitor.exe", b"monitor")]),
            zip_bytes(&[("codex-monitor.exe", b"monitor"), ("cdxm.exe", b"cdxm")]),
        ];

        for (index, archive) in cases.into_iter().enumerate() {
            let destination = TempDir::new().unwrap();
            assert!(
                extract_release_zip(&archive, destination.path()).is_err(),
                "invalid archive case {index} was accepted"
            );
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn latest_release_download_verifies_checksum_before_extracting() {
        let archive = zip_bytes(&valid_entries());
        let (address, server) = spawn_release_server(archive);

        let destination = TempDir::new().unwrap();
        let staged = download_latest_release(
            &format!("http://{address}"),
            destination.path(),
            ReleasePlatform::WindowsX64,
        )
        .await
        .unwrap();
        server.join().unwrap();
        assert_eq!(staged.len(), ManagedFile::RELEASE.len());
        assert!(destination.path().join("codex-monitor.exe").is_file());
    }

    #[tokio::test]
    async fn latest_macos_release_download_verifies_checksum_before_extracting() {
        let archive = targz_bytes(&[("codex-monitor", TarTestEntry::Regular(b"mac-monitor"))]);
        let (address, server) = spawn_release_server(archive);
        let destination = TempDir::new().unwrap();
        let staged = download_latest_release(
            &format!("http://{address}"),
            destination.path(),
            ReleasePlatform::MacArm64,
        )
        .await
        .unwrap();
        server.join().unwrap();
        assert_eq!(staged.len(), ManagedFile::RELEASE.len());
        assert_eq!(
            std::fs::read(destination.path().join("codex-monitor")).unwrap(),
            b"mac-monitor"
        );
    }
}
