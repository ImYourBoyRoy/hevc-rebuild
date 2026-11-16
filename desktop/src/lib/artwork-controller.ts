// ./desktop/src/lib/artwork-controller.ts
// Queue poster previews (library-root check, the 24-entry cache, lazy loading through an
// intersection observer) and the app artwork cache inspect/purge flow, extracted from the route
// (artwork slice). The route's state is reached through a typed `ArtworkHost`; the native bridge and
// the observer factory are injected so every branch is unit-tested in node
// (pnpm test:artwork-controller). The code is the route's code moved verbatim with `h.` prefixes.

import type { ArtworkCacheInfo, ArtworkCachePurgeResult, MediaQueueSource } from './types.js';
import { pathIsWithin, queuePathKey } from './queue-paths.ts';

export interface ObserverLike { observe(node: unknown): void; disconnect(): void }
export type ObserverFactory = (callback: (entries: { isIntersecting: boolean }[]) => void, options: { rootMargin: string }) => ObserverLike;

export interface ArtworkHost {
  queueArtworkPreviews: Record<string, string>;
  queueArtworkPreviewErrors: Record<string, string>;
  queueArtworkPreviewBusy: Record<string, boolean>;
  artworkCacheInfo: ArtworkCacheInfo | null;
  artworkCacheLoading: boolean;
  artworkCachePurging: boolean;
  artworkCacheConfirm: boolean;
  artworkCacheError: string;
  readonly queueSources: MediaQueueSource[];
  setNotice(message: string, kind?: "info" | "success" | "error"): void;
}

export interface ArtworkDeps {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  isDesktop: () => boolean;
  host: ArtworkHost;
  createObserver: ObserverFactory | null;
}

export function createArtworkController(deps: ArtworkDeps) {
  const { invoke, isDesktop, host: h, createObserver } = deps;

  function queueArtworkRoot(mediaPath: string, artworkPath: string): string | null {
    const sources = h.queueSources.filter((source) => source.scanResult?.entries.some((entry) => queuePathKey(entry.path) === queuePathKey(mediaPath)));
    const candidates = sources.length ? sources : h.queueSources;
    for (const source of candidates) {
      const scan = source.scanResult;
      if (!scan) continue;
      const separator = Math.max(scan.root.lastIndexOf("/"), scan.root.lastIndexOf("\\"));
      const root = scan.input_kind === "file" ? scan.root.slice(0, separator + 1) : scan.root;
      if (root && (pathIsWithin(root, artworkPath) || pathIsWithin(root, mediaPath))) return root;
    }
    return null;
  }

  async function ensureQueueArtworkPreview(mediaPath: string, artworkPath: string) {
    const key = queuePathKey(artworkPath);
    if (!key || h.queueArtworkPreviews[key] || h.queueArtworkPreviewBusy[key]) return;
    const root = queueArtworkRoot(mediaPath, artworkPath);
    if (!root) {
      h.queueArtworkPreviewErrors = { ...h.queueArtworkPreviewErrors, [key]: "The selected poster is outside the analyzed library." };
      return;
    }
    h.queueArtworkPreviewBusy = { ...h.queueArtworkPreviewBusy, [key]: true };
    h.queueArtworkPreviewErrors = { ...h.queueArtworkPreviewErrors, [key]: "" };
    try {
      const preview = await invoke<{ dataUrl: string }>("read_local_library_artwork_preview", {
        request: { root, path: artworkPath },
      });
      const next = { ...h.queueArtworkPreviews, [key]: preview.dataUrl };
      const previewKeys = Object.keys(next);
      while (previewKeys.length > 24) {
        const oldest = previewKeys.shift();
        if (oldest) delete next[oldest];
      }
      h.queueArtworkPreviews = next;
    } catch (cause) {
      h.queueArtworkPreviewErrors = { ...h.queueArtworkPreviewErrors, [key]: String(cause) };
    } finally {
      h.queueArtworkPreviewBusy = { ...h.queueArtworkPreviewBusy, [key]: false };
    }
  }

  function observeQueueArtwork(node: HTMLElement, params: { mediaPath: string; artworkPath: string }) {
    let current = params;
    let observer: ObserverLike | null = null;
    const observe = () => {
      observer?.disconnect();
      if (!current.artworkPath) return;
      if (!createObserver) {
        void ensureQueueArtworkPreview(current.mediaPath, current.artworkPath);
        return;
      }
      observer = createObserver((entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          observer?.disconnect();
          void ensureQueueArtworkPreview(current.mediaPath, current.artworkPath);
        }
      }, { rootMargin: "160px" });
      observer.observe(node);
    };
    observe();
    return {
      update(next: typeof params) {
        if (next.mediaPath === current.mediaPath && next.artworkPath === current.artworkPath) return;
        current = next;
        observe();
      },
      destroy() {
        observer?.disconnect();
      },
    };
  }

  async function refreshArtworkCache() {
    if (!isDesktop() || h.artworkCacheLoading || h.artworkCachePurging) return;
    h.artworkCacheLoading = true;
    h.artworkCacheError = "";
    h.artworkCacheConfirm = false;
    try {
      h.artworkCacheInfo = await invoke<ArtworkCacheInfo>("inspect_app_artwork_cache");
    } catch (error) {
      h.artworkCacheError = String(error);
    } finally {
      h.artworkCacheLoading = false;
    }
  }

  async function purgeUnusedArtworkCache() {
    if (!isDesktop() || !h.artworkCacheInfo?.purgeableImageCount || !h.artworkCacheConfirm || h.artworkCachePurging) return;
    h.artworkCachePurging = true;
    h.artworkCacheError = "";
    try {
      const result = await invoke<ArtworkCachePurgeResult>("purge_unused_app_artwork_cache", {
        previewId: h.artworkCacheInfo.previewId,
        confirmed: true,
      });
      h.artworkCacheInfo = result.remaining;
      h.artworkCacheConfirm = false;
      h.setNotice(result.message, result.failedFileCount ? "error" : "success");
    } catch (error) {
      h.artworkCacheError = String(error);
      h.artworkCacheConfirm = false;
    } finally {
      h.artworkCachePurging = false;
    }
  }

  return { queueArtworkRoot, ensureQueueArtworkPreview, observeQueueArtwork, refreshArtworkCache, purgeUnusedArtworkCache };
}
