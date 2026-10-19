# Support and troubleshooting

HEVC Rebuild Studio is source-available under the PolyForm Noncommercial 1.0.0
license. Personal and other permitted noncommercial use may use and fork the
project. Commercial use requires a separate commercial license.

## Before requesting help

Collect:

1. Operating system and architecture.
2. Application or engine version.
3. The output of `hevc-rebuild-studio json environment` or the equivalent
   `python -m hevc_rebuild --setup-only --no-dependency-repair` check.
4. The exact mode and command used.
5. The per-run report and adjacent diagnostic log.

Do not send media files, API tokens, keyring contents, private library paths,
or unredacted metadata unless they are required and you are authorized to share
them. A short redacted log excerpt is usually enough.

## Common problems

- **A tool is missing:** open the setup page or run the platform-specific setup
  command in [TOOLS.md](TOOLS.md), then re-run the read-only environment check.
- **A custom tool is not detected:** verify that the selected executable is a
  real file and executable, or clear the override and use `PATH`.
- **Subtitles or copied streams fail validation:** keep the `.error.mkv` and
  matching `.error.log` together. The output is intentionally marked
  unvalidated and should not replace the source without review.
- **MCP does not connect:** confirm the host launches the executable with
  `--mcp`, uses stdio rather than HTTP, and has permission to access only the
  intended local media scope.
- **A full rebuild is slow or stops:** review the resource forecast, available
  disk space, swap pressure, and the retained report before retrying.

## Contact

Project and commercial-license questions can be directed to
[Roy Dawson IV](mailto:Roy.Dawson.IV@gmail.com). Public issue tracking can be
added when the GitHub repository is connected; do not assume an issue response
is a guaranteed support service level.
