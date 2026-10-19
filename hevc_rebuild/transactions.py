"""Validated-output installation and rollback; native adapter and direct fallback.

Callers must complete media validation before entering these operations.
"""
from __future__ import annotations

import logging
import os
from pathlib import Path

from .util import RebuildError, fsync_dir, fsync_file, rename_no_replace, run


def commit_transaction(source: Path, temp_output: Path, final_output: Path, backup: Path) -> None:
    native_helper = os.environ.get("HEVC_REBUILD_NATIVE_HELPER")
    if native_helper:
        command = [
            native_helper,
            "internal",
            "commit-output",
            "--source",
            str(source),
            "--temporary",
            str(temp_output),
            "--final",
            str(final_output),
            "--backup",
            str(backup),
        ]
        try:
            result = run(
                command,
                timeout=300,
                check=False,
            )
        except OSError as exc:
            raise RebuildError(f"Could not start Rust's validated-output transaction: {exc}") from exc
        if result.returncode != 0:
            detail = result.stderr.strip()
            raise RebuildError(
                detail or f"Rust's validated-output transaction exited with status {result.returncode}."
            )
        return

    source_parent = source.parent.resolve(strict=True)
    if temp_output.parent.resolve(strict=True) != source_parent or final_output.parent.resolve(strict=True) != source_parent:
        raise RebuildError("Source, temporary output, and final output must share one directory")
    if source == temp_output or temp_output == final_output or backup in (source, temp_output, final_output):
        raise RebuildError("Commit paths overlap in an unsafe way")
    old_directory = source_parent / "old"
    if old_directory.is_symlink():
        raise RebuildError("The old backup directory must not be a symlink")
    created_directory = not old_directory.exists()
    old_directory.mkdir(exist_ok=True)
    if backup.parent.resolve(strict=True) != old_directory.resolve(strict=True):
        raise RebuildError("Backup directory resolves outside the source folder's old directory")
    if created_directory:
        fsync_dir(source_parent)
    fsync_file(temp_output)
    fsync_dir(temp_output.parent)
    if final_output != source and os.path.lexists(final_output):
        raise RebuildError(f"Final output already exists and will not be overwritten: {final_output}")
    if os.path.lexists(backup):
        raise RebuildError(f"Backup path already exists and will not be overwritten: {backup}")

    source_moved = False
    final_installed = False
    try:
        rename_no_replace(source, backup)
        source_moved = True
        fsync_dir(backup.parent)
        rename_no_replace(temp_output, final_output)
        final_installed = True
        fsync_dir(final_output.parent)
    except Exception as error:
        # Before the replacement file is installed, restore the original immediately.
        # If final installation already happened and only durability syncing failed, keep
        # both the new final file and the original backup rather than destroying either.
        if final_installed:
            raise RebuildError("Output was installed, but durability could not be confirmed. "
                               "The output and original backup were retained.") from error
        if source_moved and not final_installed:
            try:
                rename_no_replace(backup, source)
                fsync_dir(source.parent)
            except Exception as rollback_error:
                logging.exception("CRITICAL: automatic rollback failed; original remains at %s", backup)
                raise RebuildError(f"Commit failed; original remains preserved at {backup}. "
                                   f"Automatic rollback failed: {rollback_error}") from error
        raise


def install_output_copy(temp_output: Path, final_output: Path) -> None:
    """Install a validated separate output without moving or replacing its source."""
    native_helper = os.environ.get("HEVC_REBUILD_NATIVE_HELPER")
    if native_helper:
        command = [native_helper, "internal", "install-output-copy", "--temporary", str(temp_output), "--final", str(final_output)]
        try:
            result = run(command, timeout=300, check=False)
        except OSError as exc:
            raise RebuildError(f"Could not start Rust's no-overwrite output installer: {exc}") from exc
        if result.returncode != 0:
            raise RebuildError(result.stderr.strip() or f"Rust's output installer exited with status {result.returncode}.")
        return

    fsync_file(temp_output)
    fsync_dir(temp_output.parent)
    try:
        os.link(temp_output, final_output)
    except FileExistsError as exc:
        raise RebuildError(f"Output already exists and will not be overwritten: {final_output}") from exc
    except OSError as exc:
        raise RebuildError(f"Could not install validated output at {final_output} without overwriting existing data: {exc}") from exc
    try:
        temp_output.unlink()
        fsync_dir(final_output.parent)
    except OSError as exc:
        raise RebuildError(f"Validated output was installed at {final_output}, but temporary cleanup or directory sync failed: {exc}") from exc


