//! Rename templates, plans, previews, results and folder-scan records.

use super::*;

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RenameTemplateKind {
    Movie,
    Series,
    Season,
    Episode,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RenameTemplateInput {
    pub(crate) source_path: String,
    pub(crate) kind: RenameTemplateKind,
    pub(crate) template: String,
    pub(crate) values: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RenamePlanDocument {
    pub(crate) root: String,
    pub(crate) items: Vec<RenameTemplateInput>,
    #[serde(default)]
    pub(crate) text_blacklist_patterns: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RenamePreviewItem {
    pub(crate) source_path: String,
    pub(crate) target_path: String,
    pub(crate) target_name: String,
    pub(crate) is_directory: bool,
    pub(crate) unchanged: bool,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RenameConflict {
    pub(crate) source_path: Option<String>,
    pub(crate) target_path: String,
    pub(crate) reason: String,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RenamePreview {
    pub(crate) preview_id: String,
    pub(crate) root: String,
    pub(crate) items: Vec<RenamePreviewItem>,
    pub(crate) conflicts: Vec<RenameConflict>,
    pub(crate) can_apply: bool,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RenameApplyResult {
    pub(crate) renamed_count: usize,
    pub(crate) items: Vec<RenamePreviewItem>,
    pub(crate) message: String,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrganizerMediaFile {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) parent_path: String,
    pub(crate) extension: String,
    pub(crate) season: Option<u32>,
    pub(crate) episode: Option<u32>,
    pub(crate) multiple_episodes: bool,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrganizerSeasonDirectory {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) season: u32,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrganizerEpisodeGap {
    pub(crate) season: u32,
    pub(crate) missing_count: u32,
    pub(crate) examples: Vec<u32>,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrganizerFolderScan {
    pub(crate) root: String,
    pub(crate) rename_root: String,
    pub(crate) files: Vec<OrganizerMediaFile>,
    pub(crate) season_directories: Vec<OrganizerSeasonDirectory>,
    pub(crate) episode_gaps: Vec<OrganizerEpisodeGap>,
    pub(crate) errors: Vec<String>,
    pub(crate) truncated: bool,
}

#[derive(Clone, Debug)]
pub(super) struct PlannedItem {
    pub(super) source: PathBuf,
    pub(super) target_name: String,
    pub(super) target: PathBuf,
    pub(super) is_directory: bool,
    pub(super) unchanged: bool,
    pub(super) fingerprint: (u64, u128, bool),
}

#[derive(Clone, Debug)]
pub(super) struct InternalPlan {
    pub(super) items: Vec<PlannedItem>,
    pub(super) preview: RenamePreview,
}
