//! Artwork save requests and results.

use super::*;

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArtworkTarget {
    pub(crate) directory: String,
    pub(crate) basename: String,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArtworkSaveRequest {
    pub(crate) root: String,
    pub(crate) candidate: ArtworkCandidate,
    pub(crate) targets: Vec<ArtworkTarget>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LocalArtworkSaveRequest {
    pub(crate) root: String,
    pub(crate) source_path: String,
    pub(crate) targets: Vec<ArtworkTarget>,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavedArtworkFile {
    pub(crate) path: String,
    pub(crate) attribution_path: String,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtworkSaveResult {
    pub(crate) files: Vec<SavedArtworkFile>,
    pub(crate) bytes_per_file: usize,
    pub(crate) message: String,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalArtworkSaveResult {
    pub(crate) files: Vec<String>,
    pub(crate) bytes_per_file: usize,
    pub(crate) message: String,
}
