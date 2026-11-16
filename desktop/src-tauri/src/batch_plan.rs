//! Planning, storage forecasting, and live tracking for unattended library
//! rebuilds started through MCP.
//!
//! The flow is plan, then start. A plan records exactly which files a batch will
//! touch (path, size, modification time), how much disk the batch could need, and
//! the free-space floor it will protect. Starting a batch needs only the plan's
//! ID, and is refused if any planned file changed since. The floor is enforced by
//! the engine (preflight and live projection), not by the caller, so a weak model
//! cannot talk its way past it.
//!
//! Everything here is plain data and arithmetic so it can be tested without media.

use crate::rebuild_run::{BatchHalt, RebuildFileResult, RunStatus};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const GIB: u64 = 1024 * 1024 * 1024;
const PLAN_LIFETIME: Duration = Duration::from_secs(6 * 60 * 60);
const MAX_STORED_PLANS: usize = 8;
/// Matches the engine's per-job selection limit.
pub(crate) const MAX_PLANNED_FILES: usize = 10_000;
const RECENT_RESULTS: usize = 10;
const MAX_LISTED_FAILURES: usize = 50;

/// One source file as it was when the plan was made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlannedFile {
    pub path: PathBuf,
    pub size: u64,
    pub modified_nanos: u128,
    /// Filesystem the file lives on, so free space is judged per volume.
    pub device: u64,
}

impl PlannedFile {
    pub(crate) fn read(path: &Path) -> Result<Self, String> {
        let metadata = fs::metadata(path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        if !metadata.is_file() {
            return Err(format!("{} is not a file.", path.display()));
        }
        Ok(Self {
            path: path.to_path_buf(),
            size: metadata.len(),
            modified_nanos: modified_nanos(&metadata),
            device: device_of(&metadata),
        })
    }
}

fn modified_nanos(metadata: &fs::Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |elapsed| elapsed.as_nanos())
}

#[cfg(unix)]
fn device_of(metadata: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.dev()
}

#[cfg(not(unix))]
fn device_of(_metadata: &fs::Metadata) -> u64 {
    0
}

/// How a batch is expected to fare on one volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Fits even if every output were as large as the worst-case estimate.
    Fits,
    /// Fits only if outputs come out smaller than the worst case. The run pauses
    /// itself at the floor rather than failing.
    MayPause,
    /// Outputs would need to be much smaller than their sources to fit.
    WillPause,
}

impl Verdict {
    fn code(self) -> &'static str {
        match self {
            Self::Fits => "fits",
            Self::MayPause => "may_pause",
            Self::WillPause => "will_pause",
        }
    }
}

/// Space arithmetic for the files of one plan that live on one volume.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VolumeForecast {
    pub sample_path: PathBuf,
    pub files: usize,
    pub source_bytes: u64,
    pub free_bytes: u64,
    pub total_bytes: Option<u64>,
    pub floor_bytes: u64,
    /// Space the batch could add if every output reached the worst-case ratio.
    /// The originals are kept in a hidden originals folder, so this is net growth, not a swap.
    pub worst_case_added_bytes: u64,
    /// Largest output-to-source ratio the batch tolerates before hitting the floor.
    pub break_even_ratio: f64,
    pub verdict: Verdict,
}

/// `ratio` is the engine's worst-case output-to-source estimate.
pub(crate) fn forecast_volume(
    sample_path: PathBuf,
    files: usize,
    source_bytes: u64,
    free_bytes: u64,
    total_bytes: Option<u64>,
    floor_bytes: u64,
    ratio: f64,
) -> VolumeForecast {
    let usable = free_bytes.saturating_sub(floor_bytes);
    let worst_case_added_bytes = (source_bytes as f64 * ratio).ceil() as u64;
    let break_even_ratio = if source_bytes == 0 {
        f64::INFINITY
    } else {
        usable as f64 / source_bytes as f64
    };
    let verdict = if break_even_ratio >= ratio {
        Verdict::Fits
    } else if break_even_ratio >= 0.5 {
        Verdict::MayPause
    } else {
        Verdict::WillPause
    };
    VolumeForecast {
        sample_path,
        files,
        source_bytes,
        free_bytes,
        total_bytes,
        floor_bytes,
        worst_case_added_bytes,
        break_even_ratio,
        verdict,
    }
}

impl VolumeForecast {
    fn to_json(&self) -> Value {
        json!({
            "volume_of": self.sample_path.to_string_lossy(),
            "files": self.files,
            "source_gib": gib(self.source_bytes),
            "free_gib": gib(self.free_bytes),
            "total_gib": self.total_bytes.map(gib),
            "protected_floor_gib": gib(self.floor_bytes),
            "worst_case_added_gib": gib(self.worst_case_added_bytes),
            "break_even_output_ratio": if self.break_even_ratio.is_finite() {
                json!((self.break_even_ratio * 100.0).round() / 100.0)
            } else {
                Value::Null
            },
            "verdict": self.verdict.code(),
        })
    }

    fn sentence(&self) -> String {
        let place = self.sample_path.display();
        match self.verdict {
            Verdict::Fits => format!(
                "On the volume holding {place}: {} free, {} protected; even worst-case output ({}) fits.",
                human(self.free_bytes),
                human(self.floor_bytes),
                human(self.worst_case_added_bytes)
            ),
            Verdict::MayPause | Verdict::WillPause => format!(
                "On the volume holding {place}: {} free, {} protected. Originals are kept in a hidden .hevc-rebuild-originals folder beside each file, so this adds up to {} in the worst case. It fits only if outputs average under {:.0}% of their sources; otherwise the run pauses itself at the protected floor.",
                human(self.free_bytes),
                human(self.floor_bytes),
                human(self.worst_case_added_bytes),
                self.break_even_ratio * 100.0
            ),
        }
    }
}

/// Free space the batch must never consume: the larger of 10 GiB and 5% of the
/// volume, but never more than a quarter of it, so small disks stay usable.
pub(crate) fn default_floor_bytes(total_bytes: Option<u64>) -> u64 {
    let ten = 10 * GIB;
    match total_bytes {
        Some(total) if total > 0 => (total / 20).max(ten).min(total / 4),
        _ => ten,
    }
}

fn gib(bytes: u64) -> f64 {
    ((bytes as f64 / GIB as f64) * 100.0).round() / 100.0
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// What was decided and shown to the user before anything is written.
#[derive(Clone, Debug)]
pub(crate) struct BatchPlan {
    pub id: String,
    pub root: PathBuf,
    pub files: Vec<PlannedFile>,
    pub already_processed: usize,
    pub volumes: Vec<VolumeForecast>,
    /// Free-space floor handed to the engine for this batch, in GiB.
    pub floor_gib: f64,
    pub max_consecutive_failures: u32,
    /// Settings the engine will actually use, already resolved.
    pub settings_summary: Value,
    pub work_root: String,
    pub settings_json: String,
    pub created: Instant,
}

impl BatchPlan {
    pub(crate) fn total_source_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.size).sum()
    }

    pub(crate) fn expired(&self) -> bool {
        self.created.elapsed() > PLAN_LIFETIME
    }

    /// Compares every planned file with the disk now. Returns the first problems
    /// found (bounded), or `Ok` when nothing changed.
    pub(crate) fn verify_unchanged(&self) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        for planned in &self.files {
            match PlannedFile::read(&planned.path) {
                Ok(current) => {
                    if current.size != planned.size
                        || current.modified_nanos != planned.modified_nanos
                    {
                        problems.push(format!(
                            "{} changed after planning.",
                            planned.path.display()
                        ));
                    }
                }
                Err(_) => problems.push(format!(
                    "{} is no longer available.",
                    planned.path.display()
                )),
            }
            if problems.len() >= 10 {
                break;
            }
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }

    /// Plain-language text the model should show the user before asking for a go-ahead.
    pub(crate) fn approval_text(&self) -> String {
        let mut lines = vec![format!(
            "Rebuild {} file{} ({}) under {} to HEVC, replacing each in place.",
            self.files.len(),
            if self.files.len() == 1 { "" } else { "s" },
            human(self.total_source_bytes()),
            self.root.display()
        )];
        lines.push(
            "Each original is moved into a hidden .hevc-rebuild-originals folder beside it after the new file passes validation; nothing is deleted. Because those backups stay, disk usage goes up as the batch runs."
                .into(),
        );
        for volume in &self.volumes {
            lines.push(volume.sentence());
        }
        lines.push(format!(
            "The run pauses itself if free space reaches {} or {} files fail in a row, and you can stop it at any time.",
            human((self.floor_gib * GIB as f64) as u64),
            self.max_consecutive_failures
        ));
        if self.already_processed > 0 {
            lines.push(format!(
                "{} file{} already have a backup and will be skipped.",
                self.already_processed,
                if self.already_processed == 1 { "" } else { "s" }
            ));
        }
        lines.join("\n")
    }

    pub(crate) fn to_json(&self) -> Value {
        let preview: Vec<Value> = self
            .files
            .iter()
            .take(25)
            .map(|file| json!({"path": file.path.to_string_lossy(), "size_gib": gib(file.size)}))
            .collect();
        json!({
            "plan_id": self.id,
            "expires_in_minutes": PLAN_LIFETIME
                .saturating_sub(self.created.elapsed())
                .as_secs() / 60,
            "root": self.root.to_string_lossy(),
            "file_count": self.files.len(),
            "source_total_gib": gib(self.total_source_bytes()),
            "already_processed_skipped": self.already_processed,
            "settings": self.settings_summary,
            "work_root": self.work_root,
            "protected_free_space_gib": self.floor_gib,
            "pause_after_consecutive_failures": self.max_consecutive_failures,
            "volumes": self.volumes.iter().map(VolumeForecast::to_json).collect::<Vec<_>>(),
            "first_files": preview,
            "show_the_user": self.approval_text(),
            "next": "Show the user show_the_user, wait for their go-ahead, then call start_planned_batch with this plan_id.",
        })
    }
}

/// Remembers recent plans so a start request can only name one the server made.
#[derive(Default)]
pub(crate) struct PlanBook {
    plans: HashMap<String, BatchPlan>,
}

impl PlanBook {
    pub(crate) fn insert(&mut self, plan: BatchPlan) {
        self.plans.retain(|_, existing| !existing.expired());
        while self.plans.len() >= MAX_STORED_PLANS {
            let Some(oldest) = self
                .plans
                .iter()
                .min_by_key(|(_, plan)| plan.created)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            self.plans.remove(&oldest);
        }
        self.plans.insert(plan.id.clone(), plan);
    }

    /// A plan is single-use: starting consumes it, so one approval is one run.
    pub(crate) fn take(&mut self, id: &str) -> Result<BatchPlan, String> {
        let plan = self.plans.remove(id).ok_or_else(|| {
            "Unknown plan. Plans are single-use and expire after six hours; call plan_library_batch again."
                .to_string()
        })?;
        if plan.expired() {
            return Err("That plan has expired; call plan_library_batch again.".into());
        }
        Ok(plan)
    }

    /// Put a plan back when starting it was refused for a reason the caller can fix.
    pub(crate) fn restore(&mut self, plan: BatchPlan) {
        self.plans.insert(plan.id.clone(), plan);
    }
}

/// Live and final numbers for a running batch.
pub(crate) struct BatchTracker {
    total_files: usize,
    started: Instant,
    started_at_unix: u64,
    succeeded: usize,
    failed: usize,
    skipped: usize,
    source_bytes_done: u64,
    output_bytes_done: u64,
    recent: Vec<Value>,
    failures: Vec<Value>,
    halted: Option<BatchHalt>,
}

impl BatchTracker {
    pub(crate) fn new(total_files: usize) -> Self {
        Self {
            total_files,
            started: Instant::now(),
            started_at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs()),
            succeeded: 0,
            failed: 0,
            skipped: 0,
            source_bytes_done: 0,
            output_bytes_done: 0,
            recent: Vec::new(),
            failures: Vec::new(),
            halted: None,
        }
    }

    /// The engine learns the real file count only after discovery.
    pub(crate) fn set_total(&mut self, total_files: usize) {
        self.total_files = total_files;
    }

    pub(crate) fn record(&mut self, file: &RebuildFileResult) {
        match RunStatus::parse(&file.status) {
            Some(RunStatus::Ok) => {
                self.succeeded += 1;
                self.source_bytes_done += file.input_bytes.unwrap_or(0);
                self.output_bytes_done += file.output_bytes.unwrap_or(0);
            }
            Some(RunStatus::Skipped) => self.skipped += 1,
            Some(RunStatus::Failed | RunStatus::Interrupted) | None => {
                self.failed += 1;
                if self.failures.len() < MAX_LISTED_FAILURES {
                    self.failures
                        .push(json!({"file": file.path, "reason": file.message}));
                }
            }
            Some(RunStatus::TestOk | RunStatus::Info) => {}
        }
        self.recent.push(json!({
            "file": file.path,
            "status": file.status,
            "message": file.message,
            "source_gib": file.input_bytes.map(gib),
            "output_gib": file.output_bytes.map(gib),
            "seconds": file.elapsed_seconds.round(),
        }));
        if self.recent.len() > RECENT_RESULTS {
            self.recent.remove(0);
        }
    }

    pub(crate) fn set_halted(&mut self, halted: Option<BatchHalt>) {
        self.halted = halted;
    }

    fn finished_files(&self) -> usize {
        self.succeeded + self.failed + self.skipped
    }

    /// Seconds left, from the average pace of this batch so far.
    fn eta_seconds(&self) -> Option<u64> {
        let done = self.finished_files();
        if done == 0 || done >= self.total_files {
            return None;
        }
        let per_file = self.started.elapsed().as_secs_f64() / done as f64;
        Some((per_file * (self.total_files - done) as f64) as u64)
    }

    pub(crate) fn to_json(&self, free_bytes_now: Option<u64>) -> Value {
        json!({
            "files_total": self.total_files,
            "files_finished": self.finished_files(),
            "succeeded": self.succeeded,
            "failed": self.failed,
            "skipped": self.skipped,
            "source_bytes_rebuilt": self.source_bytes_done,
            "output_bytes_written": self.output_bytes_done,
            "originals_kept_bytes": self.source_bytes_done,
            "source_gib_rebuilt": gib(self.source_bytes_done),
            "output_gib_written": gib(self.output_bytes_done),
            "space_added_by_outputs_gib": gib(self.output_bytes_done),
            "originals_kept_gib": gib(self.source_bytes_done),
            "free_gib_now": free_bytes_now.map(gib),
            "elapsed_minutes": self.started.elapsed().as_secs() / 60,
            "eta_minutes": self.eta_seconds().map(|seconds| seconds / 60),
            "started_unix": self.started_at_unix,
            "halted": self.halted.map(|reason| json!({
                "code": reason.code(),
                "explanation": reason.explanation(),
            })),
            "failures": self.failures,
            "recent": self.recent,
        })
    }

    /// Short report for the user once the batch is over.
    pub(crate) fn final_report(
        &self,
        free_bytes_now: Option<u64>,
        stopped_by_user: bool,
    ) -> String {
        let mut lines = vec![format!(
            "{} of {} file(s) rebuilt successfully, {} failed safely, {} skipped.",
            self.succeeded, self.total_files, self.failed, self.skipped
        )];
        if self.succeeded > 0 {
            lines.push(format!(
                "New files total {} (from {} of originals). The originals are kept in hidden .hevc-rebuild-originals folders, so they are still using {} until you review and delete them.",
                human(self.output_bytes_done),
                human(self.source_bytes_done),
                human(self.source_bytes_done)
            ));
        }
        if let Some(free) = free_bytes_now {
            lines.push(format!("Free space now: {}.", human(free)));
        }
        if let Some(reason) = self.halted {
            lines.push(reason.explanation());
        } else if stopped_by_user {
            lines.push("Stopped on request; remaining files were not changed.".into());
        }
        if self.failed > 0 {
            lines.push(
                "Failed files were left as they were (or kept as .error files); see the failures list for the reason on each."
                    .into(),
            );
        }
        lines.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_with(files: Vec<PlannedFile>) -> BatchPlan {
        BatchPlan {
            id: "p1".into(),
            root: PathBuf::from("/media/lib"),
            files,
            already_processed: 0,
            volumes: Vec::new(),
            floor_gib: 20.0,
            max_consecutive_failures: 3,
            settings_summary: json!({}),
            work_root: "auto".into(),
            settings_json: "{}".into(),
            created: Instant::now(),
        }
    }

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hevc-batch-plan-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn floor_scales_with_the_volume_but_never_swallows_a_small_disk() {
        assert_eq!(default_floor_bytes(None), 10 * GIB);
        // A 4 TB volume protects 5%, well above the 10 GiB minimum.
        let four_tb = 4_000_000_000_000u64;
        assert_eq!(default_floor_bytes(Some(four_tb)), four_tb / 20);
        // A 20 GiB scratch disk would otherwise be entirely "protected".
        assert_eq!(default_floor_bytes(Some(20 * GIB)), 5 * GIB);
        assert_eq!(default_floor_bytes(Some(0)), 10 * GIB);
    }

    #[test]
    fn forecast_reports_the_break_even_ratio_and_a_plain_verdict() {
        let path = PathBuf::from("/media/lib");
        // 1 TiB of sources, 2.5 TiB free, 100 GiB protected: plenty even at 1.25x.
        let roomy = forecast_volume(
            path.clone(),
            100,
            1024 * GIB,
            2560 * GIB,
            None,
            100 * GIB,
            1.25,
        );
        assert_eq!(roomy.verdict, Verdict::Fits);
        assert!(roomy.break_even_ratio > 2.0);

        // 1 TiB of sources but only ~800 GiB usable: fine if outputs shrink to 78%.
        let tight = forecast_volume(
            path.clone(),
            100,
            1024 * GIB,
            900 * GIB,
            None,
            100 * GIB,
            1.25,
        );
        assert_eq!(tight.verdict, Verdict::MayPause);
        assert!((tight.break_even_ratio - 800.0 / 1024.0).abs() < 1e-9);

        // Outputs would have to be under 20% of their sources: the batch will pause.
        let small = forecast_volume(
            path.clone(),
            100,
            1024 * GIB,
            300 * GIB,
            None,
            100 * GIB,
            1.25,
        );
        assert_eq!(small.verdict, Verdict::WillPause);

        // Free space already below the floor leaves nothing usable, without underflow.
        let full = forecast_volume(path, 1, GIB, 5 * GIB, None, 50 * GIB, 1.25);
        assert_eq!(full.verdict, Verdict::WillPause);
        assert_eq!(full.break_even_ratio, 0.0);
    }

    #[test]
    fn approval_text_states_the_backup_growth_and_the_stop_conditions() {
        let mut plan = plan_with(vec![PlannedFile {
            path: PathBuf::from("/media/lib/A.mkv"),
            size: 8 * GIB,
            modified_nanos: 1,
            device: 1,
        }]);
        plan.volumes.push(forecast_volume(
            PathBuf::from("/media/lib"),
            1,
            8 * GIB,
            500 * GIB,
            Some(1000 * GIB),
            20 * GIB,
            1.25,
        ));
        plan.already_processed = 2;
        let text = plan.approval_text();
        assert!(text.contains("1 file"), "{text}");
        assert!(text.contains(".hevc-rebuild-originals"), "{text}");
        assert!(text.contains("nothing is deleted"), "{text}");
        assert!(text.contains("reaches 20.0 GiB"), "{text}");
        assert!(text.contains("3 files fail in a row"), "{text}");
        assert!(text.contains("2 files already have a backup"), "{text}");
        assert!(plan.to_json()["next"]
            .as_str()
            .unwrap()
            .contains("start_planned_batch"));
    }

    #[test]
    fn plan_is_refused_when_a_planned_file_changed_or_vanished() {
        let dir = scratch("verify");
        let kept = dir.join("kept.mkv");
        let grown = dir.join("grown.mkv");
        let gone = dir.join("gone.mkv");
        for path in [&kept, &grown, &gone] {
            fs::write(path, b"original bytes").unwrap();
        }
        let plan = plan_with(
            [&kept, &grown, &gone]
                .iter()
                .map(|path| PlannedFile::read(path).unwrap())
                .collect(),
        );
        assert!(plan.verify_unchanged().is_ok());
        fs::write(&grown, b"original bytes plus an edit").unwrap();
        fs::remove_file(&gone).unwrap();
        let problems = plan.verify_unchanged().unwrap_err();
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems
            .iter()
            .any(|line| line.contains("grown.mkv") && line.contains("changed")));
        assert!(problems
            .iter()
            .any(|line| line.contains("gone.mkv") && line.contains("no longer")));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_plan_can_be_used_once_and_only_if_the_server_made_it() {
        let mut book = PlanBook::default();
        assert!(book.take("invented-by-the-model").is_err());
        book.insert(plan_with(Vec::new()));
        let plan = book.take("p1").expect("first start consumes the plan");
        assert!(book.take("p1").is_err(), "an approval authorizes one run");
        book.restore(plan);
        assert!(book.take("p1").is_ok(), "a refused start can be retried");
    }

    #[test]
    fn plan_book_keeps_only_the_most_recent_plans() {
        let mut book = PlanBook::default();
        for index in 0..(MAX_STORED_PLANS + 3) {
            let mut plan = plan_with(Vec::new());
            plan.id = format!("plan-{index}");
            book.insert(plan);
        }
        assert_eq!(book.plans.len(), MAX_STORED_PLANS);
        assert!(book.take("plan-0").is_err());
        assert!(book.take(&format!("plan-{}", MAX_STORED_PLANS + 2)).is_ok());
    }

    fn result(
        status: RunStatus,
        message: &str,
        input: u64,
        output: Option<u64>,
    ) -> RebuildFileResult {
        RebuildFileResult {
            path: format!("/media/lib/{}-{input}.mkv", status.as_str()),
            status: status.as_str().into(),
            message: message.into(),
            output: None,
            elapsed_seconds: 60.0,
            input_bytes: Some(input),
            output_bytes: output,
        }
    }

    #[test]
    fn tracker_counts_space_and_keeps_failure_reasons_for_the_report() {
        let mut tracker = BatchTracker::new(5);
        tracker.record(&result(RunStatus::Ok, "", 10 * GIB, Some(4 * GIB)));
        tracker.record(&result(RunStatus::Ok, "", 6 * GIB, Some(3 * GIB)));
        tracker.record(&result(
            RunStatus::Failed,
            "Validation rejected the output",
            GIB,
            None,
        ));
        tracker.record(&result(RunStatus::Skipped, "already processed", GIB, None));
        let json = tracker.to_json(Some(100 * GIB));
        assert_eq!(json["succeeded"], 2);
        assert_eq!(json["failed"], 1);
        assert_eq!(json["skipped"], 1);
        assert_eq!(json["files_finished"], 4);
        assert_eq!(json["source_gib_rebuilt"], 16.0);
        assert_eq!(json["output_gib_written"], 7.0);
        assert_eq!(json["originals_kept_gib"], 16.0);
        assert_eq!(json["free_gib_now"], 100.0);
        // Small libraries and small test clips must not collapse to 0.0.
        let mut small = BatchTracker::new(1);
        small.record(&result(RunStatus::Ok, "", 1_800_000, Some(970_000)));
        assert_eq!(small.to_json(None)["source_bytes_rebuilt"], 1_800_000);
        assert_eq!(small.to_json(None)["originals_kept_bytes"], 1_800_000);
        assert_eq!(
            json["failures"][0]["reason"],
            "Validation rejected the output"
        );
        let report = tracker.final_report(Some(100 * GIB), false);
        assert!(report.contains("2 of 5"), "{report}");
        assert!(
            report.contains(".hevc-rebuild-originals folders"),
            "{report}"
        );
        assert!(report.contains("Free space now: 100.0 GiB"), "{report}");
    }

    #[test]
    fn tracker_reports_why_a_batch_paused_and_bounds_its_history() {
        let mut tracker = BatchTracker::new(100);
        for index in 0..30 {
            tracker.record(&result(RunStatus::Ok, "", GIB + index, Some(GIB / 2)));
        }
        tracker.set_halted(Some(BatchHalt::LowSpace));
        let json = tracker.to_json(None);
        assert_eq!(json["recent"].as_array().unwrap().len(), RECENT_RESULTS);
        assert_eq!(json["halted"]["code"], "low_space");
        assert!(tracker
            .final_report(None, false)
            .contains("free disk space ran low"));
        let mut stopped = BatchTracker::new(10);
        stopped.record(&result(RunStatus::Ok, "", GIB, Some(GIB)));
        assert!(stopped
            .final_report(None, true)
            .contains("Stopped on request"));
    }
}
