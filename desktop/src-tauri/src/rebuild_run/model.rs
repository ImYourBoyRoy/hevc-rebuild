//! Per-run and per-file working state: tools, context, stream references, resource and file plans.

use super::*;

pub(super) struct Tools {
    pub(super) ffmpeg: String,
    pub(super) ffprobe: String,
    pub(super) mediainfo: String,
    pub(super) mkvmerge: String,
    pub(super) mkvpropedit: String,
    pub(super) mkvextract: String,
    pub(super) hdr10plus: Option<String>,
    pub(super) x265_pixel_formats: Vec<String>,
    pub(super) x265_dolbyvision: bool,
    pub(super) x265_hdr10plus: bool,
}

pub(super) struct Context<'a> {
    pub(super) options: &'a RebuildRunOptions,
    pub(super) tools: &'a Tools,
    pub(super) progress: &'a mut dyn FnMut(RebuildProgress),
    pub(super) line: &'a mut dyn FnMut(String),
    pub(super) child: &'a mut dyn FnMut(Option<u32>),
}

pub(super) struct FileState {
    pub(super) phase: Phase,
    pub(super) temp_output: Option<PathBuf>,
    pub(super) final_output: Option<PathBuf>,
    pub(super) row: Map<String, Value>,
    pub(super) interrupted: bool,
}

impl FileState {
    pub(super) fn set_status(&mut self, status: RunStatus) {
        self.row.insert("status".into(), json!(status.as_str()));
    }

    /// The recorded status; a row with none counts as failed.
    pub(super) fn status(&self) -> RunStatus {
        self.row
            .get("status")
            .and_then(Value::as_str)
            .and_then(RunStatus::parse)
            .unwrap_or(RunStatus::Failed)
    }
}

pub(super) struct FileFrame<'a> {
    pub(super) source: &'a Path,
    pub(super) number: usize,
    pub(super) total: usize,
    pub(super) started: Instant,
    pub(super) input_bytes: Option<u64>,
}

#[derive(Clone)]
pub(super) struct StreamRef {
    pub(super) index: i64,
    pub(super) type_ordinal: u32,
    pub(super) codec_type: String,
    pub(super) codec_name: String,
    pub(super) width: i64,
    pub(super) height: i64,
    pub(super) default: bool,
    pub(super) attached_pic: bool,
    pub(super) disposition: Map<String, Value>,
}

impl StreamRef {
    pub(super) fn pixels(&self) -> i64 {
        self.width.max(0) * self.height.max(0)
    }

    pub(super) fn real_video(&self) -> bool {
        self.codec_type == "video" && !self.attached_pic && self.width > 0 && self.height > 0
    }

    pub(super) fn to_plan_value(&self) -> Value {
        json!({
            "index": self.index,
            "type_ordinal": self.type_ordinal,
            "codec_type": self.codec_type,
            "codec_name": self.codec_name,
            "width": self.width,
            "height": self.height,
            "default": self.default,
            "attached_pic": self.attached_pic,
            "disposition": self.disposition,
        })
    }
}

pub(super) struct ResourcePlan {
    pub(super) worker_threads: i64,
    pub(super) frame_threads: i64,
    pub(super) per_frame: f64,
    pub(super) per_worker: f64,
    pub(super) budget: Option<f64>,
    pub(super) memory_guard_gib: f64,
    pub(super) forecast: Value,
    pub(super) cpu_threads: i64,
}

impl ResourcePlan {
    pub(super) fn reduced(&self) -> Option<Self> {
        let next = retry_plan(
            self.worker_threads,
            self.frame_threads,
            self.per_frame,
            self.per_worker,
            self.budget,
        );
        if next.get("reduces_parallelism").and_then(Value::as_bool) != Some(true) {
            return None;
        }
        Some(Self {
            worker_threads: next
                .get("planned_workers")
                .and_then(Value::as_i64)
                .unwrap_or(1)
                .max(1),
            frame_threads: next
                .get("planned_frame_threads")
                .and_then(Value::as_i64)
                .unwrap_or(1)
                .max(1),
            per_frame: self.per_frame,
            per_worker: self.per_worker,
            budget: self.budget,
            memory_guard_gib: self.memory_guard_gib,
            forecast: next,
            cpu_threads: self.cpu_threads,
        })
    }
}

pub(super) struct EffectiveSettings {
    pub(super) crf: i64,
    pub(super) test_crf: i64,
    pub(super) preset: String,
    pub(super) tune: String,
    pub(super) promote_8bit_to_10bit: bool,
    pub(super) preserve_hdr10plus: bool,
    pub(super) dolby_vision_policy: String,
    pub(super) stream_indices: Option<Vec<u16>>,
    pub(super) clear_file_tune: bool,
}

// ---------------------------------------------------------------- arguments

/// Everything learned about one source before any output is written. Built by
/// `analyze_source` and filled in by the later planning stages.
pub(super) struct FilePlan {
    pub(super) effective: EffectiveSettings,
    pub(super) probe: Value,
    pub(super) mediainfo: Value,
    pub(super) profile: Value,
    pub(super) refs: Vec<StreamRef>,
    pub(super) selected_streams: Vec<StreamRef>,
    pub(super) encode_video_indices: BTreeSet<i64>,
    pub(super) real_videos: Vec<StreamRef>,
    pub(super) primary: StreamRef,
    pub(super) video_profiles: BTreeMap<i64, Value>,
    pub(super) verified_hdr10plus: BTreeSet<i64>,
    pub(super) hdr10plus_verification_skipped: bool,
    pub(super) content_tune: Option<String>,
    pub(super) preserve_dolbyvision_indices: BTreeSet<i64>,
    pub(super) strip_dolbyvision_indices: BTreeSet<i64>,
    pub(super) planning_profile: Value,
    /// Data streams skipped because Matroska cannot store them.
    pub(super) unstorable_data_streams: usize,
}

impl FilePlan {
    pub(super) fn any_hdr10plus(&self) -> bool {
        !self.verified_hdr10plus.is_empty()
    }

    pub(super) fn preserve_dolbyvision(&self) -> bool {
        !self.preserve_dolbyvision_indices.is_empty()
    }

    pub(super) fn strip_dolbyvision(&self) -> bool {
        !self.strip_dolbyvision_indices.is_empty()
    }
}

/// Whether a planning stage finished the file early (a policy skip) or the
/// pipeline should carry on to the next stage.
pub(super) enum Flow {
    Continue,
    Finished,
}

/// Where the output will be built and installed.
pub(super) struct OutputTargets {
    pub(super) final_output: PathBuf,
    pub(super) temp_output: PathBuf,
    pub(super) separate_output: bool,
    pub(super) duration: f64,
}

/// How HDR10+ dynamic metadata travels through the encode.
pub(super) struct Hdr10PlusPlan {
    /// Extracted metadata per source video stream index.
    pub(super) jsons: BTreeMap<i64, PathBuf>,
    /// libx265 lacks encode-time support, so inject after the encode.
    pub(super) post_inject: bool,
}

impl Hdr10PlusPlan {
    /// Metadata handed to the encoder itself (none when injecting afterward).
    pub(super) fn encode_jsons(&self) -> BTreeMap<i64, PathBuf> {
        if self.post_inject {
            BTreeMap::new()
        } else {
            self.jsons.clone()
        }
    }
}

/// Results of the metadata/poster embedding stage that later stages verify.
pub(super) struct EmbeddedMetadata {
    pub(super) applied: Value,
    pub(super) scrubbed: Value,
    /// Text appended to the success message (scrub results, skipped streams).
    pub(super) notes: String,
    pub(super) expect_poster: bool,
}
