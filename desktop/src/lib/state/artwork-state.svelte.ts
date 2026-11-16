// ./desktop/src/lib/state/artwork-state.svelte.ts
// Queue poster preview cache, its busy/error maps and the app artwork cache review state.
// A plain rune class: its instance is the typed host the controller works on, so the route needs no
// per-field accessors. Members marked `declare` are supplied by the route (see withGetters).
import type { ArtworkCacheInfo, MediaQueueSource } from "../types.js";

export class ArtworkState {
  queueArtworkPreviews = $state<Record<string, string>>({});
  queueArtworkPreviewErrors = $state<Record<string, string>>({});
  queueArtworkPreviewBusy = $state<Record<string, boolean>>({});
  artworkCacheInfo = $state<ArtworkCacheInfo | null>(null);
  artworkCacheLoading = $state(false);
  artworkCachePurging = $state(false);
  artworkCacheConfirm = $state(false);
  artworkCacheError = $state("");
  setNotice: (message: string, kind?: "info" | "success" | "error") => void = () => {};
  declare readonly queueSources: MediaQueueSource[];
}
