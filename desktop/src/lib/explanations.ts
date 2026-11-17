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
  return { label: "Low quality", summary: "Very small files with visible loss everywhere. Not recommended for anything you want to keep.", recommended: false };
}

/**
 * Roughly how large a file gets compared with the default CRF. A well-known rule of thumb for x265 is that
 * every 6 steps of CRF halves (or doubles) the bitrate; real results vary a lot with the video, so this is only
 * ever shown as a rough guide.
 */
export function crfSizeRatio(crf: number): number {
  const value = Math.max(0, Math.min(51, Number.isFinite(crf) ? crf : CRF_DEFAULT));
  return 2 ** ((CRF_DEFAULT - value) / 6);
}

export function crfSizeHint(crf: number): string {
  const ratio = crfSizeRatio(crf);
  const guide = " A rough guide only; real results depend on the video.";
  if (Math.abs(ratio - 1) < 0.001) return `The default: this is the baseline file size.${guide}`;
  if (ratio >= 1.15) return `About ${Number(ratio.toFixed(1))}× the size you would get at the default (CRF ${CRF_DEFAULT}).${guide}`;
  if (ratio <= 0.85) return `About ${Math.round((ratio * 100) / 5) * 5}% of the size you would get at the default (CRF ${CRF_DEFAULT}).${guide}`;
  return `Close to the size at the default (CRF ${CRF_DEFAULT}).${guide}`;
}

/** What a short test is, shown with the mode picker. It uses the run's CRF, so it previews the real result. */
export const SHORT_TEST_HELP =
  "Encodes a 5-second sample of each file with the same quality settings as the full rebuild, so its size and look preview the real result. Your original is never touched.";

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

export interface SettingHelp {
  title: string;
  text: string;
}

/** Plain-language help for settings that were only labelled in technical terms. One paragraph each. */
const SETTING_HELP: Record<string, SettingHelp> = {
  libx265: {
    title: "What is libx265?",
    text: "It is the open-source encoder that turns your video into HEVC (also called H.265), a format that keeps the same picture in a much smaller file. It runs on your computer's processor, so long films take a while.",
  },
  detectAnimation: {
    title: "Detect animation",
    text: "When the content tune is Automatic, the app looks for trustworthy signs that a file is a cartoon or anime and then applies animation tuning. Turn it off if you want Normal tuning for everything.",
  },
  hdr10plus: {
    title: "HDR10+ dynamic metadata",
    text: "HDR10+ adds brightness instructions for every scene so a compatible TV can show the picture as the creators intended. When this is on, the app keeps it automatically and checks the result afterwards. Off deliberately leaves it out; ordinary HDR still works.",
  },
  dolbyVision: {
    title: "Dolby Vision policy",
    text: "Dolby Vision is another premium HDR format that is easy to damage. Safe preservation keeps Dolby Vision that can be verified; anything unsupported is skipped, not damaged. Skip (all files, or just this one in the per-file editor) leaves Dolby Vision files alone. Verified HDR10 fallback converts to ordinary HDR10, dropping the Dolby Vision layer, only when the file has a safe HDR10 version underneath.",
  },
  secondaryVideo: {
    title: "Secondary video tracks",
    text: "A few files carry more than one video track, such as an alternate angle. Encode all selected tracks converts each one, which takes longer. Copy additional tracks keeps the extra ones exactly as they are. Keep primary track only leaves the extras out.",
  },
  dataStreams: {
    title: "Data streams",
    text: "Some files carry extra tracks that are neither picture nor sound, for example timecode. Preserve keeps them in the new file. Drop leaves them out, which is fine for normal viewing.",
  },
  copyVerification: {
    title: "Verify copied audio and subtitles",
    text: "Audio and subtitles are never re-encoded; they are copied across untouched. This option then reads the copies back and compares a fingerprint (SHA-256) of each with the original to prove they are identical. It adds a little time and is worth leaving on.",
  },
  qualityCheck: {
    title: "Picture quality check",
    text: "After encoding, the app compares sampled scenes of the new file with the original using SSIM, a score from 0 to 1 where 1 means identical. Enforce keeps your original and rejects the new file if the score falls below the minimum. Report only records the scores. Off skips the check.",
  },
  minSsim: {
    title: "Minimum SSIM",
    text: "The lowest SSIM score the new file may get compared with the original (1 means identical). 0.95 catches broken output without being fussy; a higher number is stricter. Very grainy films score lower even when they look fine, so do not set it too high for them.",
  },
  youtubeBitrateProfile: {
    title: "Bitrate profile",
    text: "Bitrate is how much data each second of video gets. Balanced sizes the upload copy from your original's bitrate, using the size target below. YouTube bitrate guidance uses the rates YouTube itself recommends, which can make much larger files.",
  },
  youtubeSizeTarget: {
    title: "Balanced size target",
    text: "How much video data the upload copy gets compared with your original: 2× means about twice the original's bitrate. It is a target, not a strict size limit, and YouTube's recommended rate is the ceiling.",
  },
  youtubeShrink: {
    title: "Shrink oversized files",
    text: "Optimize keeps healthy files as they are and only re-encodes ones that are far bigger than YouTube needs. Auto re-encodes only when a file is far above YouTube's guidance. Off always copies and never re-encodes. Aggressive re-encodes anything above the guidance.",
  },
  youtubeQualityCheck: {
    title: "Quality check after re-encode",
    text: "A re-encoded file is accepted only if sampled scenes score at least the minimum SSIM against the original (a score from 0 to 1; 1 is identical). Strict needs 0.990, Standard 0.985, Relaxed 0.975. Off trusts the encoder without checking.",
  },
  youtubeSdrCodec: {
    title: "SDR codec",
    text: "The video format for normal (non-HDR) uploads. H.264 works everywhere and is the safe default. HEVC makes smaller files, but YouTube may take longer to process them. HDR videos always use HEVC.",
  },
};

export const SETTING_HELP_KEYS = Object.keys(SETTING_HELP);

export function settingHelp(key: string): SettingHelp {
  return SETTING_HELP[key] ?? { title: "About this setting", text: "This setting is explained in the app's Help section." };
}
