#![deny(unsafe_code)]

use flate2::read::GzDecoder;
use fs2::FileExt as _;
use futures_util::StreamExt as _;
use reqwest::header::{ACCEPT, USER_AGENT};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use tempfile::{NamedTempFile, TempDir};
use thiserror::Error;

const DEFAULT_REPOSITORY: &str = "woosal1337/bot";
const GITHUB_API: &str = "https://api.github.com";
const GITHUB_API_VERSION: &str = "2022-11-28";
const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_CHECKSUM_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateMode {
    Check,
    Install,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateOutcome {
    Current { version: Version },
    LocalNewer { current: Version, latest: Version },
    Available { current: Version, latest: Version },
    Updated { previous: Version, current: Version },
    Staged { previous: Version, current: Version },
}

impl UpdateOutcome {
    pub fn message(&self) -> String {
        match self {
            Self::Current { version } => format!("Bot {version} is current."),
            Self::LocalNewer { current, latest } => format!(
                "Bot {current} is newer than the latest published release, {latest}. No update was installed."
            ),
            Self::Available { current, latest } => {
                format!("Bot {latest} is available. You have {current}. Run `bot update`.")
            }
            Self::Updated { previous, current } => format!(
                "Updated Bot from {previous} to {current}. Existing Bot sessions keep {previous} until they restart."
            ),
            Self::Staged { previous, current } => format!(
                "Staged Bot {current} over {previous}. The update will finish after this command exits."
            ),
        }
    }
}

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("Bot cannot parse the installed version `{0}`.")]
    InvalidCurrentVersion(String),
    #[error("Bot cannot parse the release tag `{0}`.")]
    InvalidReleaseVersion(String),
    #[error("Bot does not publish updates for this platform: {0}.")]
    UnsupportedPlatform(String),
    #[error("The latest GitHub release is not stable.")]
    UnstableRelease,
    #[error("The release does not contain `{0}`.")]
    MissingAsset(String),
    #[error("The release asset `{0}` does not have a SHA-256 digest.")]
    MissingAssetDigest(String),
    #[error("The release asset `{name}` is too large: {size} bytes.")]
    AssetTooLarge { name: String, size: u64 },
    #[error(
        "The downloaded size for `{name}` is {actual} bytes, but GitHub reports {expected} bytes."
    )]
    AssetSizeMismatch {
        name: String,
        expected: u64,
        actual: u64,
    },
    #[error("The SHA-256 digest for `{0}` does not match the GitHub release metadata.")]
    AssetDigestMismatch(String),
    #[error("The checksum file does not contain `{0}`.")]
    MissingChecksum(String),
    #[error("The SHA-256 digest for `{0}` does not match `SHA256SUMS`.")]
    ChecksumMismatch(String),
    #[error("The archive does not contain `{0}`.")]
    MissingBinary(String),
    #[error("The staged Bot binary reports version {actual}, but the release tag is {expected}.")]
    StagedVersionMismatch { expected: Version, actual: Version },
    #[error("Another Bot update is active.")]
    UpdateLocked,
    #[error("Bot could not contact the release service: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Bot could not read or write an update file: {0}")]
    Io(#[from] io::Error),
    #[error("Bot could not read the release archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("Bot could not parse the release response: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Bot could not keep the staged update: {0}")]
    Persist(#[from] tempfile::PersistError),
    #[error("Bot could not replace the installed binary: {0}")]
    PathPersist(#[from] tempfile::PathPersistError),
    #[error("The staged Bot binary did not start: {0}")]
    StagedBinary(io::Error),
    #[error("The staged Bot binary returned invalid version data.")]
    InvalidStagedVersion,
    #[error("Bot could not start the Windows update helper: {0}")]
    WindowsHelper(io::Error),
}

#[derive(Clone, Debug, Deserialize)]
struct ReleaseResponse {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<ReleaseAsset>,
}

#[derive(Clone, Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArchiveFormat {
    TarGz,
    Zip,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Platform {
    target: String,
    format: ArchiveFormat,
}

impl Platform {
    fn current() -> Result<Self, UpdateError> {
        Self::from_parts(std::env::consts::OS, std::env::consts::ARCH)
    }

    fn from_parts(os: &str, arch: &str) -> Result<Self, UpdateError> {
        let architecture = match arch {
            "x86_64" => "x86_64",
            "aarch64" => "aarch64",
            _ => return Err(UpdateError::UnsupportedPlatform(format!("{os}/{arch}"))),
        };
        let (system, format) = match os {
            "linux" => ("unknown-linux-gnu", ArchiveFormat::TarGz),
            "macos" => ("apple-darwin", ArchiveFormat::TarGz),
            "windows" if architecture == "x86_64" => ("pc-windows-msvc", ArchiveFormat::Zip),
            _ => return Err(UpdateError::UnsupportedPlatform(format!("{os}/{arch}"))),
        };
        Ok(Self {
            target: format!("{architecture}-{system}"),
            format,
        })
    }

    fn archive_name(&self) -> String {
        let extension = match self.format {
            ArchiveFormat::TarGz => "tar.gz",
            ArchiveFormat::Zip => "zip",
        };
        format!("bot-{}.{}", self.target, extension)
    }

    fn binary_entry(&self) -> String {
        let suffix = if self.format == ArchiveFormat::Zip {
            ".exe"
        } else {
            ""
        };
        format!("bot-{}/bot{suffix}", self.target)
    }
}

struct Release {
    version: Version,
    archive: ReleaseAsset,
    checksums: ReleaseAsset,
}

pub async fn execute(
    current_version: &str,
    mode: UpdateMode,
) -> Result<UpdateOutcome, UpdateError> {
    let current = parse_version(current_version)
        .ok_or_else(|| UpdateError::InvalidCurrentVersion(current_version.to_owned()))?;
    let repository = std::env::var("BOT_REPOSITORY").unwrap_or_else(|_| DEFAULT_REPOSITORY.into());
    let platform = Platform::current()?;
    let client = xai_grok_extra_ca::build_reqwest_client(|builder| {
        builder
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
    })?;
    let release = fetch_release(&client, &repository, &platform).await?;
    if current == release.version {
        return Ok(UpdateOutcome::Current { version: current });
    }
    if current > release.version {
        return Ok(UpdateOutcome::LocalNewer {
            current,
            latest: release.version,
        });
    }
    if mode == UpdateMode::Check {
        return Ok(UpdateOutcome::Available {
            current,
            latest: release.version,
        });
    }
    install_release(&client, current, release, platform).await
}

async fn fetch_release(
    client: &reqwest::Client,
    repository: &str,
    platform: &Platform,
) -> Result<Release, UpdateError> {
    let url = format!("{GITHUB_API}/repos/{repository}/releases/latest");
    let response = client
        .get(url)
        .header(ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
        .header(USER_AGENT, "bot-updater")
        .send()
        .await?
        .error_for_status()?;
    let response: ReleaseResponse = response.json().await?;
    resolve_release(response, platform)
}

fn resolve_release(response: ReleaseResponse, platform: &Platform) -> Result<Release, UpdateError> {
    if response.draft || response.prerelease {
        return Err(UpdateError::UnstableRelease);
    }
    let version_text = response.tag_name.trim_start_matches('v');
    let version = Version::parse(version_text)
        .map_err(|_| UpdateError::InvalidReleaseVersion(response.tag_name.clone()))?;
    let archive_name = platform.archive_name();
    let archive = response
        .assets
        .iter()
        .find(|asset| asset.name == archive_name)
        .cloned()
        .ok_or_else(|| UpdateError::MissingAsset(archive_name))?;
    let checksums = response
        .assets
        .iter()
        .find(|asset| asset.name == "SHA256SUMS")
        .cloned()
        .ok_or_else(|| UpdateError::MissingAsset("SHA256SUMS".into()))?;
    Ok(Release {
        version,
        archive,
        checksums,
    })
}

async fn install_release(
    client: &reqwest::Client,
    previous: Version,
    release: Release,
    platform: Platform,
) -> Result<UpdateOutcome, UpdateError> {
    let executable = std::env::current_exe()?;
    let install_directory = executable.parent().ok_or_else(|| {
        UpdateError::Io(io::Error::other("the executable has no parent directory"))
    })?;
    let lock_path = install_directory.join(".bot-update.lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.try_lock_exclusive()
        .map_err(|_| UpdateError::UpdateLocked)?;
    let downloads = TempDir::new()?;
    let checksum_path = downloads.path().join("SHA256SUMS");
    let archive_path = downloads.path().join(&release.archive.name);
    let checksum_digest = download_asset(
        client,
        &release.checksums,
        &checksum_path,
        MAX_CHECKSUM_BYTES,
    )
    .await?;
    verify_api_digest(&release.checksums, &checksum_digest)?;
    let archive_digest =
        download_asset(client, &release.archive, &archive_path, MAX_ARCHIVE_BYTES).await?;
    verify_api_digest(&release.archive, &archive_digest)?;
    let checksum_text = fs::read_to_string(checksum_path)?;
    let expected = checksum_for(&checksum_text, &release.archive.name)
        .ok_or_else(|| UpdateError::MissingChecksum(release.archive.name.clone()))?;
    if !expected.eq_ignore_ascii_case(&archive_digest) {
        return Err(UpdateError::ChecksumMismatch(release.archive.name));
    }
    let mut staged = NamedTempFile::new_in(install_directory)?;
    extract_binary(
        &archive_path,
        &platform,
        staged.as_file_mut(),
        &release.archive.name,
    )?;
    staged.as_file_mut().flush()?;
    set_executable(staged.path())?;
    staged.as_file().sync_all()?;
    let staged_version = read_binary_version(staged.path())?;
    if staged_version != release.version {
        return Err(UpdateError::StagedVersionMismatch {
            expected: release.version,
            actual: staged_version,
        });
    }
    apply_update(staged, &executable, &previous, &release.version)
}

async fn download_asset(
    client: &reqwest::Client,
    asset: &ReleaseAsset,
    destination: &Path,
    maximum: u64,
) -> Result<String, UpdateError> {
    if asset.size > maximum {
        return Err(UpdateError::AssetTooLarge {
            name: asset.name.clone(),
            size: asset.size,
        });
    }
    let response = client
        .get(&asset.browser_download_url)
        .header(USER_AGENT, "bot-updater")
        .send()
        .await?
        .error_for_status()?;
    let mut stream = response.bytes_stream();
    let mut file = File::create(destination)?;
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        size = size.saturating_add(chunk.len() as u64);
        if size > maximum {
            return Err(UpdateError::AssetTooLarge {
                name: asset.name.clone(),
                size,
            });
        }
        file.write_all(&chunk)?;
        digest.update(&chunk);
    }
    file.sync_all()?;
    if size != asset.size {
        return Err(UpdateError::AssetSizeMismatch {
            name: asset.name.clone(),
            expected: asset.size,
            actual: size,
        });
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn verify_api_digest(asset: &ReleaseAsset, actual: &str) -> Result<(), UpdateError> {
    let expected = asset
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .ok_or_else(|| UpdateError::MissingAssetDigest(asset.name.clone()))?;
    if !expected.eq_ignore_ascii_case(actual) {
        return Err(UpdateError::AssetDigestMismatch(asset.name.clone()));
    }
    Ok(())
}

fn checksum_for<'a>(contents: &'a str, name: &str) -> Option<&'a str> {
    contents.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let digest = fields.next()?;
        let file_name = fields.next()?.trim_start_matches('*');
        (file_name == name && fields.next().is_none()).then_some(digest)
    })
}

fn extract_binary(
    archive_path: &Path,
    platform: &Platform,
    destination: &mut File,
    archive_name: &str,
) -> Result<(), UpdateError> {
    let expected = platform.binary_entry();
    match platform.format {
        ArchiveFormat::TarGz => {
            let archive = File::open(archive_path)?;
            let decoder = GzDecoder::new(archive);
            let mut archive = tar::Archive::new(decoder);
            for entry in archive.entries()? {
                let mut entry = entry?;
                if entry.path()?.as_ref() == Path::new(&expected)
                    && entry.header().entry_type().is_file()
                {
                    io::copy(&mut entry, destination)?;
                    return Ok(());
                }
            }
        }
        ArchiveFormat::Zip => {
            let archive = File::open(archive_path)?;
            let mut archive = zip::ZipArchive::new(archive)?;
            if let Ok(mut entry) = archive.by_name(&expected) {
                if !entry.is_file() {
                    return Err(UpdateError::MissingBinary(expected));
                }
                io::copy(&mut entry, destination)?;
                return Ok(());
            }
        }
    }
    Err(UpdateError::MissingBinary(format!(
        "{archive_name}:{expected}"
    )))
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<(), UpdateError> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(windows)]
fn set_executable(_path: &Path) -> Result<(), UpdateError> {
    Ok(())
}

fn read_binary_version(path: &Path) -> Result<Version, UpdateError> {
    let output = Command::new(path)
        .args(["version", "--json"])
        .output()
        .map_err(UpdateError::StagedBinary)?;
    if !output.status.success() {
        return Err(UpdateError::InvalidStagedVersion);
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct VersionOutput {
        current_version: String,
    }
    let value: VersionOutput = serde_json::from_slice(&output.stdout)?;
    parse_version(&value.current_version).ok_or(UpdateError::InvalidStagedVersion)
}

fn parse_version(value: &str) -> Option<Version> {
    value
        .trim()
        .trim_start_matches('v')
        .split_whitespace()
        .next()
        .and_then(|value| Version::parse(value).ok())
}

#[cfg(unix)]
fn apply_update(
    staged: NamedTempFile,
    executable: &Path,
    previous: &Version,
    current: &Version,
) -> Result<UpdateOutcome, UpdateError> {
    staged.into_temp_path().persist(executable)?;
    if let Some(parent) = executable.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(UpdateOutcome::Updated {
        previous: previous.clone(),
        current: current.clone(),
    })
}

#[cfg(windows)]
fn apply_update(
    staged: NamedTempFile,
    executable: &Path,
    previous: &Version,
    current: &Version,
) -> Result<UpdateOutcome, UpdateError> {
    use std::process::Stdio;

    let (_, staged_path) = staged.keep()?;
    let script = "$ErrorActionPreference='Stop'; $parent=[int]$args[0]; $current=$args[1]; $staged=$args[2]; Wait-Process -Id $parent -ErrorAction SilentlyContinue; $backup=\"$current.bot-old\"; try { if (Test-Path -LiteralPath $backup) { Remove-Item -LiteralPath $backup -Force }; Move-Item -LiteralPath $current -Destination $backup -Force; Move-Item -LiteralPath $staged -Destination $current -Force; Remove-Item -LiteralPath $backup -Force } catch { if ((Test-Path -LiteralPath $backup) -and -not (Test-Path -LiteralPath $current)) { Move-Item -LiteralPath $backup -Destination $current -Force }; exit 1 }";
    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            script,
        ])
        .arg(std::process::id().to_string())
        .arg(executable)
        .arg(staged_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(UpdateError::WindowsHelper)?;
    Ok(UpdateOutcome::Staged {
        previous: previous.clone(),
        current: current.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> ReleaseAsset {
        ReleaseAsset {
            name: name.into(),
            browser_download_url: format!("https://example.test/{name}"),
            size: 10,
            digest: Some(format!("sha256:{}", "a".repeat(64))),
        }
    }

    #[test]
    fn selects_exact_platform_assets() {
        let platform = Platform::from_parts("linux", "x86_64").expect("supported platform");
        let response = ReleaseResponse {
            tag_name: "v1.2.3".into(),
            draft: false,
            prerelease: false,
            assets: vec![asset(&platform.archive_name()), asset("SHA256SUMS")],
        };
        let release = resolve_release(response, &platform).expect("valid release");
        assert_eq!(release.version, Version::new(1, 2, 3));
        assert_eq!(release.archive.name, "bot-x86_64-unknown-linux-gnu.tar.gz");
    }

    #[test]
    fn rejects_prerelease_response() {
        let platform = Platform::from_parts("linux", "x86_64").expect("supported platform");
        let response = ReleaseResponse {
            tag_name: "v2.0.0-beta.1".into(),
            draft: false,
            prerelease: true,
            assets: vec![],
        };
        assert!(matches!(
            resolve_release(response, &platform),
            Err(UpdateError::UnstableRelease)
        ));
    }

    #[test]
    fn platform_matrix_matches_release_names() {
        let cases = [
            ("linux", "x86_64", "bot-x86_64-unknown-linux-gnu.tar.gz"),
            ("linux", "aarch64", "bot-aarch64-unknown-linux-gnu.tar.gz"),
            ("macos", "x86_64", "bot-x86_64-apple-darwin.tar.gz"),
            ("macos", "aarch64", "bot-aarch64-apple-darwin.tar.gz"),
            ("windows", "x86_64", "bot-x86_64-pc-windows-msvc.zip"),
        ];
        for (os, arch, expected) in cases {
            let platform = Platform::from_parts(os, arch).expect("supported platform");
            assert_eq!(platform.archive_name(), expected);
        }
        assert!(Platform::from_parts("windows", "aarch64").is_err());
    }

    #[test]
    fn checksum_parser_requires_an_exact_file_name() {
        let checksums = "aaaa  bot-x86_64-unknown-linux-gnu.tar.gz.old\nbbbb *bot-x86_64-unknown-linux-gnu.tar.gz\n";
        assert_eq!(
            checksum_for(checksums, "bot-x86_64-unknown-linux-gnu.tar.gz"),
            Some("bbbb")
        );
    }

    #[test]
    fn version_parser_accepts_release_build_output() {
        assert_eq!(
            parse_version("1.0.2 (29b536a66882)"),
            Some(Version::new(1, 0, 2))
        );
        assert_eq!(parse_version("v1.2.3"), Some(Version::new(1, 2, 3)));
        assert_eq!(parse_version("invalid"), None);
    }

    #[test]
    fn extracts_only_the_expected_tar_entry() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let archive_path = directory.path().join("bot.tar.gz");
        let archive = File::create(&archive_path).expect("archive file");
        let encoder = flate2::write::GzEncoder::new(archive, flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);
        let platform = Platform::from_parts("linux", "x86_64").expect("supported platform");
        let payload = b"staged-bot";
        let mut header = tar::Header::new_gnu();
        header.set_size(payload.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        archive
            .append_data(&mut header, platform.binary_entry(), &payload[..])
            .expect("archive entry");
        archive
            .into_inner()
            .expect("gzip encoder")
            .finish()
            .expect("gzip archive");
        let mut staged = NamedTempFile::new_in(directory.path()).expect("staged file");
        extract_binary(&archive_path, &platform, staged.as_file_mut(), "bot.tar.gz")
            .expect("extract binary");
        assert_eq!(fs::read(staged.path()).expect("staged bytes"), payload);
    }

    #[test]
    fn extracts_only_the_expected_zip_entry() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let archive_path = directory.path().join("bot.zip");
        let archive = File::create(&archive_path).expect("archive file");
        let mut archive = zip::ZipWriter::new(archive);
        let platform = Platform::from_parts("windows", "x86_64").expect("supported platform");
        archive
            .start_file(
                platform.binary_entry(),
                zip::write::SimpleFileOptions::default(),
            )
            .expect("archive entry");
        archive.write_all(b"staged-bot").expect("archive bytes");
        archive.finish().expect("zip archive");
        let mut staged = NamedTempFile::new_in(directory.path()).expect("staged file");
        extract_binary(&archive_path, &platform, staged.as_file_mut(), "bot.zip")
            .expect("extract binary");
        assert_eq!(
            fs::read(staged.path()).expect("staged bytes"),
            b"staged-bot"
        );
    }

    #[test]
    fn asset_digest_must_match_github_metadata() {
        let archive = asset("bot.tar.gz");
        assert!(verify_api_digest(&archive, &"a".repeat(64)).is_ok());
        assert!(matches!(
            verify_api_digest(&archive, &"b".repeat(64)),
            Err(UpdateError::AssetDigestMismatch(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn unix_install_replaces_the_binary_atomically() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let executable = directory.path().join("bot");
        fs::write(&executable, b"old").expect("old binary");
        let mut staged = NamedTempFile::new_in(directory.path()).expect("staged file");
        staged.write_all(b"new").expect("staged binary");
        let outcome = apply_update(
            staged,
            &executable,
            &Version::new(1, 0, 2),
            &Version::new(1, 0, 3),
        )
        .expect("apply update");
        assert_eq!(fs::read(executable).expect("installed binary"), b"new");
        assert_eq!(
            outcome,
            UpdateOutcome::Updated {
                previous: Version::new(1, 0, 2),
                current: Version::new(1, 0, 3),
            }
        );
    }

    #[test]
    fn outcome_messages_give_one_next_action() {
        let outcome = UpdateOutcome::Available {
            current: Version::new(1, 0, 2),
            latest: Version::new(1, 1, 0),
        };
        assert_eq!(
            outcome.message(),
            "Bot 1.1.0 is available. You have 1.0.2. Run `bot update`."
        );
    }
}
