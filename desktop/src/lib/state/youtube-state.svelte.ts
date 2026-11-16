// ./desktop/src/lib/state/youtube-state.svelte.ts
// YouTube preparation choices and the HDR10+ loss approvals.
// A plain rune class: its instance is the typed host the controller works on, so the route needs no
// per-field accessors. Members marked `declare` are supplied by the route (see withGetters).
import type { YoutubeMode, YoutubeSdrCodec, YoutubePreset, YoutubeShrink, YoutubeQualityCheck } from "../types.js";

export class YoutubeState {
  youtubeResolution = $state<"source" | "1440p" | "4k">("1440p");
  youtubeQualityMode = $state<"balanced" | "recommended">("balanced");
  youtubeSourceMultiplier = $state(2);
  youtubeMode = $state<YoutubeMode>("upscale");
  youtubeSdrCodec = $state<YoutubeSdrCodec>("h264");
  youtubePreset = $state<YoutubePreset>("slow");
  youtubeShrink = $state<YoutubeShrink>("auto");
  youtubeQualityCheck = $state<YoutubeQualityCheck>("standard");
  youtubeOutputDir = $state("");
  youtubeHdr10plusLossPaths = $state<Set<string>>(new Set());
}
