use rmcp::schemars;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::CString;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use unicode_normalization::UnicodeNormalization;

mod apply;
mod cli;
mod plan;
mod scan;
mod template;
mod types;

#[cfg(test)]
use apply::*;
pub(crate) use apply::{apply_rename_batch_with_patterns, rename_without_replacing};
pub(crate) use cli::organizer_cli_mode;
pub(crate) use plan::preview_rename_batch_with_patterns;
use plan::*;
pub(crate) use scan::scan_organizer_folder;
use scan::*;
use template::*;
use types::*;
pub(crate) use types::{
    OrganizerEpisodeGap, OrganizerFolderScan, OrganizerMediaFile, OrganizerSeasonDirectory,
    RenameApplyResult, RenameConflict, RenamePlanDocument, RenamePreview, RenamePreviewItem,
    RenameTemplateInput, RenameTemplateKind,
};

const MAX_PLAN_ITEMS: usize = 2_000;
const MAX_SCAN_FILES: usize = 10_000;
const MAX_TEMPLATE_BYTES: usize = 1_024;
const MAX_PLAN_JSON_BYTES: usize = 2 * 1024 * 1024;
const MAX_FILENAME_BYTES: usize = 240;
const MAX_WALK_DEPTH: usize = 24;

#[cfg(test)]
mod tests;
