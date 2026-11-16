//! File phases, run and batch status, the failure-streak halt policy, and progress and result records.

use super::*;

/// The stage a file is in. The label is shown in failure messages and written to
/// retained-output logs, so it must stay stable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    MediaAnalysis,
    StoragePreflight,
    Hdr10PlusExtraction,
    HardwareCalibration,
    VideoEncode,
    Hdr10PlusInject,
    MetadataEmbedding,
    OutputValidation,
    ValidatedOutputInstall,
    TransactionCommit,
}

impl Phase {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::MediaAnalysis => "media analysis",
            Self::StoragePreflight => "storage preflight",
            Self::Hdr10PlusExtraction => "HDR10+ metadata extraction",
            Self::HardwareCalibration => "hardware calibration",
            Self::VideoEncode => "video encode",
            Self::Hdr10PlusInject => "HDR10+ metadata inject",
            Self::MetadataEmbedding => "metadata embedding",
            Self::OutputValidation => "output validation",
            Self::ValidatedOutputInstall => "validated output install",
            Self::TransactionCommit => "transaction commit",
        }
    }

    /// From this phase on, a complete encoded file exists, so a failure keeps it
    /// for review as `.error.mkv` instead of discarding it.
    pub(super) fn holds_finished_output(self) -> bool {
        matches!(
            self,
            Self::MetadataEmbedding
                | Self::OutputValidation
                | Self::ValidatedOutputInstall
                | Self::TransactionCommit
        )
    }

    /// How far through the file's progress bar a failure in this phase lands.
    pub(super) fn failure_fraction(self) -> f64 {
        match self {
            Self::MetadataEmbedding => 0.90,
            Self::OutputValidation => 0.97,
            Self::ValidatedOutputInstall | Self::TransactionCommit => 0.99,
            _ => 0.0,
        }
    }
}

/// Outcome of one file, stored in the report row's `status` column. The text is
/// part of the CSV report format and of `RebuildFileResult.status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunStatus {
    Ok,
    TestOk,
    Info,
    Skipped,
    Failed,
    Interrupted,
}

impl RunStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::TestOk => "TEST_OK",
            Self::Info => "INFO",
            Self::Skipped => "SKIPPED",
            Self::Failed => "FAILED",
            Self::Interrupted => "INTERRUPTED",
        }
    }

    pub(crate) fn parse(text: &str) -> Option<Self> {
        [
            Self::Ok,
            Self::TestOk,
            Self::Info,
            Self::Skipped,
            Self::Failed,
            Self::Interrupted,
        ]
        .into_iter()
        .find(|status| status.as_str() == text)
    }

    /// A file that ended without error, even if it did no work (skipped/info).
    pub(super) fn is_success(self) -> bool {
        matches!(self, Self::Ok | Self::TestOk | Self::Info | Self::Skipped)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RebuildProgress {
    pub current: usize,
    pub total: usize,
    pub file_name: String,
    pub file_path: String,
    pub phase: String,
    pub file_percent: Option<f64>,
    pub queue_percent: f64,
    pub elapsed_seconds: f64,
    pub speed: Option<f64>,
    pub eta_seconds: Option<f64>,
    pub input_bytes: Option<u64>,
    pub output_bytes: Option<u64>,
    pub estimated_output_bytes: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RebuildFileResult {
    pub path: String,
    pub status: String,
    pub message: String,
    pub output: Option<String>,
    pub elapsed_seconds: f64,
    /// Source size before the run touched it.
    pub input_bytes: Option<u64>,
    /// Size of the validated output that now exists, when there is one.
    pub output_bytes: Option<u64>,
}

/// Why a multi-file run ended before it reached the end of its list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BatchHalt {
    /// The disk is too full to keep going safely. Remaining files were not touched.
    LowSpace,
    /// This many files in a row failed, which points at a systemic problem.
    ConsecutiveFailures(u32),
}

impl BatchHalt {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::LowSpace => "low_space",
            Self::ConsecutiveFailures(_) => "consecutive_failures",
        }
    }

    pub(crate) fn explanation(self) -> String {
        match self {
            Self::LowSpace => "Paused because free disk space ran low. Remaining files were not changed; free some space and run the same batch again to continue.".into(),
            Self::ConsecutiveFailures(count) => format!(
                "Paused after {count} files failed in a row, which usually means a systemic problem rather than bad media. Remaining files were not changed."
            ),
        }
    }
}

/// Phrases the engine uses when storage is the reason a file could not proceed.
/// A test pins both producers to this list.
const LOW_SPACE_MARKERS: [&str; 2] = ["Insufficient free space", "Stopped to protect disk space"];

pub(super) fn is_low_space_failure(message: &str) -> bool {
    LOW_SPACE_MARKERS
        .iter()
        .any(|marker| message.contains(marker))
}

/// Decides, file by file, whether an unattended run should keep going.
pub(super) struct HaltPolicy {
    max_consecutive_failures: u32,
    consecutive_failures: u32,
}

impl HaltPolicy {
    pub(super) fn new(max_consecutive_failures: u32) -> Self {
        Self {
            max_consecutive_failures,
            consecutive_failures: 0,
        }
    }

    pub(super) fn observe(&mut self, outcome: RunStatus, message: &str) -> Option<BatchHalt> {
        match outcome {
            RunStatus::Ok | RunStatus::TestOk | RunStatus::Info => self.consecutive_failures = 0,
            RunStatus::Skipped | RunStatus::Interrupted => {}
            RunStatus::Failed => {
                if is_low_space_failure(message) {
                    return Some(BatchHalt::LowSpace);
                }
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
                if self.max_consecutive_failures > 0
                    && self.consecutive_failures >= self.max_consecutive_failures
                {
                    return Some(BatchHalt::ConsecutiveFailures(
                        self.max_consecutive_failures,
                    ));
                }
            }
        }
        None
    }
}

pub(crate) struct RebuildJobResult {
    pub files: Vec<RebuildFileResult>,
    pub log_path: PathBuf,
    pub report_path: PathBuf,
    pub stop_requested: bool,
    pub failed: usize,
    pub succeeded: usize,
    pub skipped: usize,
    /// Set when the run stopped itself to protect the disk or the library.
    pub halted: Option<BatchHalt>,
}
