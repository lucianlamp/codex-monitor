use super::model::{sha256_bytes, sha256_file, ManagedFile, StagedFile};
use anyhow::{bail, Context};
use std::{
    collections::BTreeSet,
    fs::File,
    io::{Cursor, Write},
    path::Path,
};
use zip::ZipArchive;

pub const WINDOWS_ARCHIVE: &str = "codex-monitor-x86_64-pc-windows-msvc.zip";
const MAX_ARCHIVE_BYTES: usize = 128 * 1024 * 1024;

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
        let id = ManagedFile::from_release_name(&name)
            .with_context(|| format!("release ZIP contains an unexpected file: {name}"))?;
        if !seen.insert(id) {
            bail!("release ZIP contains a duplicate file: {name}");
        }

        let output_path = destination.join(id.staged_name());
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
                required.staged_name()
            );
        }
    }
    Ok(staged)
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

#[cfg(windows)]
pub async fn download_latest_release(
    release_base: &str,
    destination: &Path,
) -> anyhow::Result<Vec<StagedFile>> {
    let base = release_base.trim_end_matches('/');
    let archive_url = format!("{base}/{WINDOWS_ARCHIVE}");
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
    extract_release_zip(&archive, destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read, Write};
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
        vec![("codex-monitor.exe", b"monitor"), ("cdxm.exe", b"cdxm")]
    }

    fn duplicate_name_zip() -> Vec<u8> {
        let mut archive = zip_bytes(&[
            ("codex-monitor.exe", b"one"),
            ("codex-monitor.tmp", b"two"),
            ("cdxm.exe", b"cdxm"),
        ]);
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
        assert_eq!(declared_zip_entry_count(&archive).unwrap(), 3);
        archive
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
        assert_eq!(staged.len(), 2);
        assert_eq!(
            std::fs::read(destination.path().join("cdxm.exe")).unwrap(),
            b"cdxm"
        );
    }

    #[test]
    fn release_zip_rejects_missing_duplicate_nested_and_unexpected_members() {
        let cases = [
            zip_bytes(&valid_entries()[..1]),
            duplicate_name_zip(),
            zip_bytes(&[
                ("nested/codex-monitor.exe", b"monitor"),
                ("cdxm.exe", b"cdxm"),
            ]),
            zip_bytes(&[
                ("codex-monitor.exe", b"monitor"),
                ("cdxm.exe", b"cdxm"),
                ("unexpected.exe", b"bad"),
            ]),
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
        let checksum = sha256_bytes(&archive);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_archive = archive.clone();
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
                    server_archive.as_slice()
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

        let destination = TempDir::new().unwrap();
        let staged = download_latest_release(&format!("http://{address}"), destination.path())
            .await
            .unwrap();
        server.join().unwrap();
        assert_eq!(staged.len(), ManagedFile::RELEASE.len());
        assert!(destination.path().join("codex-monitor.exe").is_file());
    }
}
