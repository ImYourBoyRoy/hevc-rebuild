// ./desktop/src/lib/explanations.ts
// Plain-language help for the encoder settings, written for people who have never heard of CRF,
// presets or film grain. Pure data and small functions so every claim is unit-tested and the Run
// screen, per-file editor and advisories all say the same thing.

export const CRF_DEFAULT = 21;

export interface CrfBand {
  label: string;
  summary: string;
  recommended: boolean;
}

/** What a CRF number means in practice. Lower numbers keep more detail and make bigger files. */
export function crfBand(crf: number): CrfBand {
  const value = Math.max(0, Math.min(51, Math.round(Number.isFinite(crf) ? crf : CRF_DEFAULT)));
  if (value <= 17) return { label: "Near-lossless", summary: "Keeps almost everything, including noise. Files can be larger than the original. Rarely worth it.", recommended: false };
  if (value <= 20) return { label: "Very high quality", summary: "Fine detail is kept. Large files, a good choice for archiving.", recommended: false };
  if (value <= 22) return { label: "Recommended", summary: "Looks the same as the original to most people, with a good saving. The default is 21.", recommended: true };
  if (value <= 24) return { label: "Smaller files", summary: "Noticeably smaller. Still excellent for most films; dark scenes and grain may look slightly softer.", recommended: false };
  if (value <= 28) return { label: "Space saver", summary: "Much smaller. Expect softer detail and banding in dark or grainy scenes.", recommended: false };
  return { label: "Low quality", summary: "Very small. Visible loss everywhere. Only for previews or tests.", recommended: false };
}

export const CRF_EXPLANATION =
  "CRF is one quality dial for the whole video. A lower number keeps more detail and makes a bigger file; a higher number discards fine detail to make a smaller file. Typical choices are 18 to 26.";

export interface TuneHelp {
  title: string;
  summary: string;
  detail: string;
}

const TUNES: Record<string, TuneHelp> = {
  auto: {
    title: "Automatic",
    summary: "Applies animation tuning only when the file is reliably known to be animated.",
    detail: "Automatic looks for trustworthy animation information in the file. Everything else gets no special tuning. Automatic never turns on film grain; that is always your explicit choice.",
  },
  animation: {
    title: "Animation",
    summary: "For cartoons and anime: flat colors and sharp lines.",
    detail: "Tunes the encoder for flat areas of color and crisp edges, which are common in cartoons, anime and CGI. Do not use it for live-action films.",
  },
  grain: {
    title: "Film grain",
    summary: "Keeps every speck of grain. Files stay much larger.",
    detail: "Film grain is the fine, random speckle you see in footage shot on film, especially in dark scenes. Because it is random, it is very expensive to store: this setting spends extra data keeping it, so files can come out far larger than without it. Use it only when you want to preserve the grain look exactly. It is the wrong choice if your goal is to shrink a dark or grainy film.",
  },
  none: {
    title: "Normal",
    summary: "General-purpose settings for live-action video.",
    detail: "No special tuning. The encoder treats grain as detail it may soften slightly, which is what gives the best size savings on most films.",
  },
};

export function tuneHelp(tune: string): TuneHelp {
  return TUNES[tune] ?? TUNES.auto;
}

const PRESETS: Record<string, string> = {
  ultrafast: "Fastest, but files come out much larger. Only for quick tests.",
  superfast: "Very fast, but files are noticeably larger.",
  veryfast: "Fast with larger files than slower presets.",
  faster: "Faster than normal; files are somewhat larger.",
  fast: "A little faster than normal; files are slightly larger.",
  medium: "Balanced. Quicker than slow, with slightly larger files.",
  slow: "Recommended. Takes more time to find a smaller file at the same quality.",
  slower: "Slower again for a small extra saving.",
  veryslow: "Much slower for a very small extra saving. Rarely worth the wait.",
  placebo: "Extremely slow with almost no benefit. Avoid.",
};

export function presetHelp(preset: string): string {
  return PRESETS[preset] ?? "Compression effort: slower presets take longer and make smaller files at the same quality.";
}

export interface Advisory {
  level: "info" | "warning";
  text: string;
}

/** Warnings shown beside the settings so a surprising choice is explained before a long run. */
export function encoderAdvisories(options: { tune: string; crf: number }): Advisory[] {
  const advisories: Advisory[] = [];
  if (options.tune === "grain") {
    advisories.push({
      level: "warning",
      text: "Film grain keeps the grain texture, so dark or grainy films can stay close to their original size or come out larger than with Normal. In our test on a dark 4K film, it made the file about three times larger than Normal at the same quality setting. Choose Normal or Automatic if you want the biggest saving.",
    });
  }
  if (options.crf <= 17) {
    advisories.push({ level: "info", text: "CRF this low produces very large files, sometimes larger than the original." });
  } else if (options.crf >= 27) {
    advisories.push({ level: "info", text: "CRF this high loses detail: expect soft images and banding in dark scenes." });
  }
  return advisories;
}
