// ./desktop/src/lib/explanations.ts
// Plain-language help for the encoder settings, written for people who have never heard of CRF,
// presets or film grain. The words live in help-content.json, which the MCP server embeds too
// (`explain_settings`, `app_guide`), so the app and a model are always told the same thing. This
// module holds the small functions around that data so every claim is unit-tested.

import content from "./help-content.json" with { type: "json" };

export const CRF_DEFAULT: number = content.crf.default;

export interface CrfBand {
  label: string;
  summary: string;
  recommended: boolean;
}

/** The CRF bands as stored: each applies up to and including `max`. */
export const CRF_BAND_TABLE: Array<CrfBand & { max: number }> = content.crf.bands;

/** What a CRF number means in practice. Lower numbers keep more detail and make bigger files. */
export function crfBand(crf: number): CrfBand {
  const value = Math.max(0, Math.min(51, Math.round(Number.isFinite(crf) ? crf : CRF_DEFAULT)));
  const band = CRF_BAND_TABLE.find((candidate) => value <= candidate.max) ?? CRF_BAND_TABLE[CRF_BAND_TABLE.length - 1];
  return { label: band.label, summary: band.summary, recommended: band.recommended };
}

/**
 * Roughly how large a file gets compared with the default CRF. A well-known rule of thumb for x265 is that
 * every 6 steps of CRF halves (or doubles) the bitrate; real results vary a lot with the video, so this is only
 * ever shown as a rough guide.
 */
export function crfSizeRatio(crf: number): number {
  const value = Math.max(0, Math.min(51, Number.isFinite(crf) ? crf : CRF_DEFAULT));
  return 2 ** ((CRF_DEFAULT - value) / content.crf.stepsToHalveSize);
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
export const SHORT_TEST_HELP: string = content.shortTest;

export const CRF_EXPLANATION: string = content.crf.explanation;

export interface TuneHelp {
  title: string;
  summary: string;
  detail: string;
}

const TUNES: Record<string, TuneHelp> = content.tunes;

export function tuneHelp(tune: string): TuneHelp {
  return TUNES[tune] ?? TUNES.auto;
}

const PRESETS: Record<string, string> = content.presets;

export function presetHelp(preset: string): string {
  return PRESETS[preset] ?? content.presetFallback;
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
const SETTING_HELP: Record<string, SettingHelp> = content.settings;

export const SETTING_HELP_KEYS = Object.keys(SETTING_HELP);

export function settingHelp(key: string): SettingHelp {
  return SETTING_HELP[key] ?? { title: "About this setting", text: "This setting is explained in the app's Help section." };
}

export interface GuideItem {
  title: string;
  text: string;
}

export interface GuideSection {
  title: string;
  summary: string;
  items: GuideItem[];
}

const GUIDE: Record<string, GuideSection> = content.guide;

/** The sections of the application guide: workflow, modes, safety, headless and MCP use. */
export const GUIDE_SECTIONS = Object.keys(GUIDE);

export function guideSection(key: string): GuideSection {
  return GUIDE[key] ?? { title: "", summary: "", items: [] };
}
