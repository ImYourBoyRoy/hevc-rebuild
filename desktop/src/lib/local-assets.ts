// ./desktop/src/lib/local-assets.ts
// Local artwork discovery extracted from MetadataWorkbench (T10.7 slice 2): scanning folders for artwork beside media, resetting
// when the library root changes, ignoring stale scans, looking up a file's or folder's assets and loading poster previews.
// The native bridge is injected so every branch is unit-tested in node (pnpm test:local-assets). The code is the component's
// code moved verbatim; the component still owns the state through a getter/setter host.

import type { Attachment } from "svelte/attachments";
import type { LocalArtworkFile, LocalAssetsScan, LocalDirectoryAssets, LocalMediaAssets, OrganizerFile } from "./metadata-workbench-types.js";
import { normalizedPath } from "./organizer-analysis.ts";

export interface LocalAssetsHost {
  localAssets: LocalAssetsScan | null;
  localAssetScanError: string;
  localAssetGeneration: number;
  localArtworkPreviews: Record<string, string>;
  localArtworkPreviewErrors: Record<string, string>;
  localArtworkPreviewBusy: Record<string, boolean>;
}

export interface LocalAssetsDeps {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  isDesktop: () => boolean;
  host: LocalAssetsHost;
}

export function createLocalAssets(deps: LocalAssetsDeps) {
  const { host: h, invoke, isDesktop } = deps;

  async function ensureLocalAssets(root: string, files: OrganizerFile[], directories: string[] = []): Promise<LocalAssetsScan | null> {
    if (!isDesktop()) return null;
    let rootKey = normalizedPath(root);
    const containsPath = (parent: string, child: string) => {
      const normalizedParent = parent.replace(/\/$/, "");
      return child === parent || child.startsWith(`${normalizedParent}/`);
    };
    if (h.localAssets && containsPath(normalizedPath(h.localAssets.root), rootKey)) {
      rootKey = normalizedPath(h.localAssets.root);
    } else if (h.localAssets && !containsPath(rootKey, normalizedPath(h.localAssets.root))) {
      h.localAssets = null;
      h.localAssetGeneration += 1;
      h.localArtworkPreviews = {};
      h.localArtworkPreviewErrors = {};
    } else if (h.localAssets && containsPath(rootKey, normalizedPath(h.localAssets.root))
      && normalizedPath(h.localAssets.root) !== rootKey) {
      h.localAssets = null;
      h.localAssetGeneration += 1;
      h.localArtworkPreviews = {};
      h.localArtworkPreviewErrors = {};
    }
    if (!h.localAssets) {
      h.localAssets = { root: rootKey, files: [], directories: [] };
      h.localAssetGeneration += 1;
    }
    const generation = h.localAssetGeneration;
    const knownFiles = new Set(h.localAssets.files.map((entry) => normalizedPath(entry.path)));
    const knownDirectories = new Set(h.localAssets.directories.map((entry) => normalizedPath(entry.path)));
    const mediaPaths = files.map((file) => file.path).filter((path) => !knownFiles.has(normalizedPath(path)));
    const directoryPaths = [...new Set(directories.map(normalizedPath))].filter((path) => !knownDirectories.has(path));
    if (!mediaPaths.length && !directoryPaths.length) return h.localAssets;
    h.localAssetScanError = "";
    try {
      const found = await invoke<LocalAssetsScan>("scan_local_library_assets", {
        request: { root: rootKey, mediaPaths, directoryPaths },
      });
      if (generation !== h.localAssetGeneration) return null;
      const mergedFiles = new Map(h.localAssets?.files.map((entry) => [normalizedPath(entry.path), entry]) ?? []);
      const mergedDirectories = new Map(h.localAssets?.directories.map((entry) => [normalizedPath(entry.path), entry]) ?? []);
      for (const entry of found.files) mergedFiles.set(normalizedPath(entry.path), entry);
      for (const entry of found.directories) mergedDirectories.set(normalizedPath(entry.path), entry);
      h.localAssets = { root: found.root, files: [...mergedFiles.values()], directories: [...mergedDirectories.values()] };
      return h.localAssets;
    } catch (cause) {
      if (generation === h.localAssetGeneration) h.localAssetScanError = String(cause);
      return null;
    }
  }

  function localFileAssets(path: string): LocalMediaAssets | undefined {
    return h.localAssets?.files.find((entry) => normalizedPath(entry.path) === normalizedPath(path));
  }

  function localDirectoryAssets(path: string): LocalDirectoryAssets | undefined {
    return h.localAssets?.directories.find((entry) => normalizedPath(entry.path) === normalizedPath(path));
  }

  function previewLocalArtwork(images: LocalArtworkFile[]): Attachment<HTMLElement> {
    return () => {
      void loadLocalArtworkPreviews(images);
    };
  }

  async function loadLocalArtworkPreviews(images: LocalArtworkFile[]) {
    if (!isDesktop() || !h.localAssets) return;
    for (const image of images.slice(0, 4)) {
      if (h.localArtworkPreviews[image.path] || h.localArtworkPreviewBusy[image.path]) continue;
      h.localArtworkPreviewBusy = { ...h.localArtworkPreviewBusy, [image.path]: true };
      h.localArtworkPreviewErrors = { ...h.localArtworkPreviewErrors, [image.path]: "" };
      try {
        const preview = await invoke<{ dataUrl: string }>("read_local_library_artwork_preview", {
          request: { root: h.localAssets.root, path: image.path },
        });
        h.localArtworkPreviews = { ...h.localArtworkPreviews, [image.path]: preview.dataUrl };
      } catch (cause) {
        h.localArtworkPreviewErrors = { ...h.localArtworkPreviewErrors, [image.path]: String(cause) };
      } finally {
        h.localArtworkPreviewBusy = { ...h.localArtworkPreviewBusy, [image.path]: false };
      }
    }
  }

  return { ensureLocalAssets, localFileAssets, localDirectoryAssets, previewLocalArtwork, loadLocalArtworkPreviews };
}
