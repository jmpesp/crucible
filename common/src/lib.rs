// Copyright 2021 Oxide Computer Company
use std::fmt;
use std::fs::File;
use std::hash::Hasher;
use std::io::{ErrorKind, Read, Write};
use std::path::Path;

use ErrorKind::NotFound;

use anyhow::{anyhow, bail, Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use slog::Drain;
use tempfile::NamedTempFile;
use tokio::time::Duration;

mod region;
pub use region::{
    config_path, Block, BlockIndex, BlockOffset, ExtentId, RegionDefinition,
    RegionOptions, DATABASE_READ_VERSION, DATABASE_WRITE_VERSION,
    MAX_BLOCK_SIZE, MAX_SHIFT, MIN_BLOCK_SIZE, MIN_SHIFT,
};

pub mod impacted_blocks;
pub mod x509;

pub const REPAIR_PORT_OFFSET: u16 = 4000;

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(
    thiserror::Error,
    Debug,
    PartialEq,
    Clone,
    Serialize,
    Deserialize,
    JsonSchema,
)]
pub enum CrucibleError {
    #[error("Error: {0}")]
    GenericError(String),

    #[error("IO Error: {0}")]
    IoError(String),

    #[error("Unsupported: {0}")]
    Unsupported(String),

    #[error("data store disconnected")]
    Disconnect,

    #[error("Error grabbing data lock")]
    DataLockError,

    #[error("Error grabbing reader-writer {0} lock")]
    RwLockError(String),

    #[error("BlockOpWaiter recv channel disconnected")]
    RecvDisconnected,

    #[error("SendError: {0}")]
    SendError(String),

    #[error("Offset not block aligned")]
    OffsetUnaligned,

    #[error("Data length not block size multiple")]
    DataLenUnaligned,

    #[error("Block size mismatch")]
    BlockSizeMismatch,

    #[error("Invalid number of blocks: {0}")]
    InvalidNumberOfBlocks(String),

    #[error("Offset past end of extent")]
    OffsetInvalid,

    #[error("Upstairs activation is in progress")]
    UpstairsActivateInProgress,

    #[error("Upstairs is deactivating")]
    UpstairsDeactivating,

    #[error("Upstairs is not active")]
    UpstairsInactive,

    #[error("Saw a UUID that wasn't ours!")]
    UuidMismatch,

    #[error("Encryption failed! {0}")]
    EncryptionError(String),

    #[error("Decryption failed!")]
    DecryptionError,

    #[error("Integrity hash mismatch!")]
    HashMismatch,

    #[error("LBA range overlap!")]
    LBARangeOverlap,

    #[error("Subvolume size mismatch!")]
    SubvolumeSizeMismatch,

    #[error("Subvolume type mismatch!")]
    SubvolumeTypeMismatch,

    #[error("Cannot serve blocks: {0}")]
    CannotServeBlocks(String),

    #[error("Cannot receive blocks: {0}")]
    CannotReceiveBlocks(String),

    #[error("Snapshot failed! {0}")]
    SnapshotFailed(String),

    #[error("Snapshot {0} exists already")]
    SnapshotExistsAlready(String),

    #[error("Attempting to modify read-only region!")]
    ModifyingReadOnlyRegion,

    #[error("Invalid extent")]
    InvalidExtent,

    #[error("Repair request error {0}")]
    RepairRequestError(String),

    #[error("Invalid repair file list {0}")]
    RepairFilesInvalid(String),

    #[error("Repair stream error {0}")]
    RepairStreamError(String),

    #[error("Active with different generation number")]
    GenerationNumberInvalid,

    #[error("No longer active")]
    NoLongerActive,

    #[error("Failed reconciliation")]
    RegionAssembleError,

    #[error("Property not available: {0}")]
    PropertyNotAvailable(String),

    #[error("Invalid downstairs replace {0}")]
    ReplaceRequestInvalid(String),

    #[error("missing context slot for block {0}")]
    MissingContextSlot(u64),

    #[error("metadata deserialization failed: {0}")]
    BadMetadata(String),

    #[error("context slot deserialization failed: {0}")]
    BadContextSlot(String),

    #[error("missing block context for non-empty block")]
    MissingBlockContext,

    #[error("Incompatible RegionDefinition {0}")]
    RegionIncompatible(String),

    #[error("Negotiation error: {0}")]
    NegotiationError(NegotiationError),
}

impl From<std::io::Error> for CrucibleError {
    fn from(e: std::io::Error) -> Self {
        CrucibleError::IoError(format!("{:?}", e))
    }
}

impl From<CrucibleError> for std::io::Error {
    fn from(e: CrucibleError) -> Self {
        std::io::Error::new(std::io::ErrorKind::Other, e)
    }
}

impl From<anyhow::Error> for CrucibleError {
    fn from(e: anyhow::Error) -> Self {
        CrucibleError::GenericError(format!("{:?}", e))
    }
}

impl<T> From<std::sync::mpsc::SendError<T>> for CrucibleError {
    fn from(e: std::sync::mpsc::SendError<T>) -> Self {
        CrucibleError::GenericError(format!("{:?}", e))
    }
}

impl From<NegotiationError> for CrucibleError {
    fn from(value: NegotiationError) -> Self {
        CrucibleError::NegotiationError(value)
    }
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(
    thiserror::Error,
    Debug,
    PartialEq,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    JsonSchema,
)]
pub enum NegotiationError {
    #[error("Message received out of order")]
    OutOfOrder,

    #[error("Generation 0 is illegal")]
    GenerationZeroIsIllegal,

    #[error(
        "Generation number is too low: requested {requested}, found {actual}"
    )]
    GenerationNumberTooLow { requested: u64, actual: u64 },

    #[error("Incompatible message version: wanted {expected}, got {actual}")]
    IncompatibleVersion { expected: u32, actual: u32 },

    #[error(
        "Incompatible encryption settings: wanted {expected}, got {actual}"
    )]
    EncryptionMismatch { expected: bool, actual: bool },

    #[error(
        "Incompatible read-only settings: wanted {expected}, got {actual}"
    )]
    ReadOnlyMismatch { expected: bool, actual: bool },

    #[error("Incompatible upstairs ID: wanted {expected}, got {actual}")]
    UpstairsIdMismatch {
        expected: uuid::Uuid,
        actual: uuid::Uuid,
    },

    #[error("Incompatible session ID: wanted {expected}, got {actual}")]
    SessionIdMismatch {
        expected: uuid::Uuid,
        actual: uuid::Uuid,
    },
}

#[macro_export]
macro_rules! crucible_bail {
    ($i:ident) => { return Err(CrucibleError::$i) };
    ($i:ident, $str:expr) => {
        return Err(CrucibleError::$i($str.to_string()))
    };
    ($i:ident, $fmt:expr, $($arg:tt)*) => {
        return Err(CrucibleError::$i(format!($fmt, $($arg)*)))
    };
}

pub fn read_json_maybe<P, T>(file: P) -> Result<Option<T>>
where
    P: AsRef<Path>,
    for<'de> T: Deserialize<'de>,
{
    let file = file.as_ref();
    let mut f = match File::open(file) {
        Ok(f) => f,
        Err(e) if e.kind() == NotFound => return Ok(None),
        Err(e) => bail!("open {:?}: {:?}", file, e),
    };
    let mut buf = Vec::<u8>::new();
    f.read_to_end(&mut buf)
        .with_context(|| anyhow!("read {:?}", file))?;
    serde_json::from_slice(buf.as_slice())
        .with_context(|| anyhow!("parse {:?}", file))
}

pub fn read_json<P, T>(file: P) -> Result<T>
where
    P: AsRef<Path>,
    for<'de> T: Deserialize<'de>,
{
    let file = file.as_ref();
    read_json_maybe(file)?
        .ok_or_else(|| anyhow!("open {:?}: file not found", file))
}

pub fn write_json<P, T>(file: P, data: &T, clobber: bool) -> Result<()>
where
    P: AsRef<Path>,
    T: Serialize,
{
    let file = file.as_ref();
    let mut buf = serde_json::to_vec_pretty(data)?;
    buf.push(b'\n');
    let mut tmpf = NamedTempFile::new_in(file.parent().unwrap())?;
    tmpf.write_all(&buf)?;
    tmpf.flush()?;

    if clobber {
        tmpf.persist(file)?;
    } else {
        tmpf.persist_noclobber(file)?;
    }
    Ok(())
}

pub fn mkdir_for_file(file: &Path) -> Result<()> {
    Ok(std::fs::create_dir_all(file.parent().expect("file path"))?)
}

pub fn integrity_hash(args: &[&[u8]]) -> u64 {
    let mut hasher: twox_hash::XxHash64 = Default::default();
    for arg in args {
        hasher.write(arg);
    }
    hasher.finish()
}

/// Detailed build information about Crucible.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct BuildInfo {
    pub version: String,
    pub git_sha: String,
    pub git_commit_timestamp: String,
    pub git_branch: String,
    pub rustc_semver: String,
    pub rustc_channel: String,
    pub rustc_host_triple: String,
    pub rustc_commit_sha: String,
    pub cargo_triple: String,
    pub debug: bool,
    pub opt_level: u8,
}

impl Default for BuildInfo {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            git_sha: env!("VERGEN_GIT_SHA").to_string(),
            git_commit_timestamp: env!("VERGEN_GIT_COMMIT_TIMESTAMP")
                .to_string(),
            git_branch: env!("VERGEN_GIT_BRANCH").to_string(),
            rustc_semver: env!("VERGEN_RUSTC_SEMVER").to_string(),
            rustc_channel: env!("VERGEN_RUSTC_CHANNEL").to_string(),
            rustc_host_triple: env!("VERGEN_RUSTC_HOST_TRIPLE").to_string(),
            rustc_commit_sha: env!("VERGEN_RUSTC_COMMIT_HASH").to_string(),
            cargo_triple: env!("VERGEN_CARGO_TARGET_TRIPLE").to_string(),
            debug: env!("VERGEN_CARGO_DEBUG").parse().unwrap(),
            opt_level: env!("VERGEN_CARGO_OPT_LEVEL").parse().unwrap(),
        }
    }
}

impl std::fmt::Display for BuildInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Crucible Version: {}\n\
            Commit SHA: {}\n\
            Commit timestamp: {}  branch: {}\n\
            rustc: {} {} {}\n\
            Cargo: {}  Debug: {} Opt level: {}",
            self.version,
            self.git_sha,
            self.git_commit_timestamp,
            self.git_branch,
            self.rustc_semver,
            self.rustc_channel,
            self.rustc_host_triple,
            self.cargo_triple,
            self.debug,
            self.opt_level
        )
    }
}

/**
 * A common logger setup for all to use.
 */
pub fn build_logger() -> slog::Logger {
    build_logger_with_level(slog::Level::Info)
}

/// Build a logger with the specific log level
pub fn build_logger_with_level(level: slog::Level) -> slog::Logger {
    let main_drain = if atty::is(atty::Stream::Stdout) {
        let decorator = slog_term::TermDecorator::new().build();
        let drain = slog_term::FullFormat::new(decorator).build().fuse();
        slog_async::Async::new(drain)
            .overflow_strategy(slog_async::OverflowStrategy::Block)
            .build_no_guard()
    } else {
        let drain = slog_bunyan::with_name("crucible", std::io::stdout())
            .build()
            .fuse();
        slog_async::Async::new(drain)
            .overflow_strategy(slog_async::OverflowStrategy::Block)
            .build_no_guard()
    };

    let (dtrace_drain, probe_reg) = slog_dtrace::Dtrace::new();

    let filtered_main = slog::LevelFilter::new(main_drain, level);

    let log = slog::Logger::root(
        slog::Duplicate::new(filtered_main.fuse(), dtrace_drain.fuse()).fuse(),
        slog::o!(),
    );

    if let slog_dtrace::ProbeRegistration::Failed(err) = probe_reg {
        slog::error!(&log, "Error registering slog-dtrace probes: {:?}", err);
    }

    log
}

impl From<CrucibleError> for dropshot::HttpError {
    fn from(e: CrucibleError) -> Self {
        match e {
            CrucibleError::BlockSizeMismatch
            | CrucibleError::DataLenUnaligned
            | CrucibleError::InvalidNumberOfBlocks(_)
            | CrucibleError::ModifyingReadOnlyRegion
            | CrucibleError::OffsetInvalid
            | CrucibleError::OffsetUnaligned
            | CrucibleError::RegionIncompatible(_)
            | CrucibleError::ReplaceRequestInvalid(_)
            | CrucibleError::SnapshotExistsAlready(_)
            | CrucibleError::Unsupported(_) => {
                dropshot::HttpError::for_bad_request(None, e.to_string())
            }

            CrucibleError::IoError(_)
            | CrucibleError::SnapshotFailed(_)
            | CrucibleError::UpstairsInactive => {
                dropshot::HttpError::for_unavail(None, e.to_string())
            }

            CrucibleError::CannotReceiveBlocks(_)
            | CrucibleError::CannotServeBlocks(_)
            | CrucibleError::DataLockError
            | CrucibleError::DecryptionError
            | CrucibleError::Disconnect
            | CrucibleError::EncryptionError(_)
            | CrucibleError::GenerationNumberInvalid
            | CrucibleError::GenericError(_)
            | CrucibleError::HashMismatch
            | CrucibleError::InvalidExtent
            | CrucibleError::LBARangeOverlap
            | CrucibleError::NoLongerActive
            | CrucibleError::PropertyNotAvailable(_)
            | CrucibleError::RecvDisconnected
            | CrucibleError::RegionAssembleError
            | CrucibleError::RepairFilesInvalid(_)
            | CrucibleError::RepairRequestError(_)
            | CrucibleError::RepairStreamError(_)
            | CrucibleError::RwLockError(_)
            | CrucibleError::SendError(_)
            | CrucibleError::SubvolumeSizeMismatch
            | CrucibleError::SubvolumeTypeMismatch
            | CrucibleError::UpstairsActivateInProgress
            | CrucibleError::UpstairsDeactivating
            | CrucibleError::UuidMismatch
            | CrucibleError::MissingContextSlot(..)
            | CrucibleError::BadMetadata(..)
            | CrucibleError::BadContextSlot(..)
            | CrucibleError::MissingBlockContext
            | CrucibleError::NegotiationError(..) => {
                dropshot::HttpError::for_internal_error(e.to_string())
            }
        }
    }
}

/// Helper object for verbose timeouts
///
/// This object waits for `tick * count`, printing a log message every `tick`
pub struct VerboseTimeout {
    /// Interval at which we log a timeout warning
    pub tick: Duration,
    /// Number of intervals before returning
    pub count: u32,
}

impl VerboseTimeout {
    /// Wait for `tick * count` duration, logging every `tick`
    pub async fn wait(&self, log: &slog::Logger) {
        for i in 0..self.count {
            tokio::time::sleep(self.tick).await;
            slog::warn!(log, "timeout {}/{}", i + 1, self.count);
        }
    }

    /// Returns the total timeout duration for this object
    pub fn timeout(&self) -> Duration {
        self.tick * self.count
    }
}
