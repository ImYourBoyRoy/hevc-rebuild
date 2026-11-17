//! The fixed Ubuntu package allowlist shared by the GUI, CLI and MCP one-pass setup.

pub(crate) const APT_ALLOWLIST: &[&str] = &[
    "ffmpeg",
    "mediainfo",
    "mkvtoolnix",
    "fontconfig",
    "ca-certificates",
];
