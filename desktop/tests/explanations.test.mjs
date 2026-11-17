// ./desktop/tests/explanations.test.mjs
// Plain-language help for encoder settings and the advisories shown beside them. Run: pnpm test:explanations

import assert from "node:assert/strict";
import test from "node:test";
import {
  crfBand, crfSizeHint, crfSizeRatio, encoderAdvisories, presetHelp, tuneHelp, settingHelp, SETTING_HELP_KEYS, SHORT_TEST_HELP, CRF_DEFAULT,
  GUIDE_SECTIONS, guideSection, CRF_BAND_TABLE,
} from "../src/lib/explanations.ts";
import { readFileSync } from "node:fs";

test("every CRF from 0 to 51 maps to exactly one named band, and the default is the recommended one", () => {
  const labels = new Set();
  for (let crf = 0; crf <= 51; crf += 1) {
    const band = crfBand(crf);
    assert.ok(band.label && band.summary, `CRF ${crf} has a band`);
    labels.add(band.label);
  }
  assert.ok(labels.size >= 5, "several distinct bands, not one blanket message");
  assert.equal(CRF_DEFAULT, 21);
  assert.equal(crfBand(CRF_DEFAULT).recommended, true);
  assert.equal(crfBand(40).recommended, false);
  assert.equal(crfBand(-5).label, crfBand(0).label, "out-of-range values clamp");
  assert.equal(crfBand(99).label, crfBand(51).label);
});

test("each tune has a name, a one-line summary and a plain explanation that does not assume jargon", () => {
  for (const tune of ["auto", "animation", "grain", "none"]) {
    const help = tuneHelp(tune);
    assert.ok(help.title && help.summary && help.detail, tune);
  }
  const grain = tuneHelp("grain");
  assert.match(grain.detail, /grain is the fine, random speckle/i);
  assert.match(grain.detail, /larger/i);
  assert.match(tuneHelp("auto").detail, /never turns on film grain/i);
  assert.equal(tuneHelp("bogus").title, tuneHelp("auto").title, "unknown values read as automatic");
});

test("presets explain the speed versus size trade and flag the recommended one", () => {
  assert.match(presetHelp("slow"), /recommended/i);
  assert.match(presetHelp("ultrafast"), /larger/i);
  assert.match(presetHelp("veryslow"), /slower/i);
  assert.ok(presetHelp("unknown-preset").length > 0);
});

test("film grain warns that dark and grainy films will stay large, at any CRF", () => {
  const [advisory] = encoderAdvisories({ tune: "grain", crf: 21 });
  assert.equal(advisory.level, "warning");
  assert.match(advisory.text, /dark/i);
  assert.match(advisory.text, /larger/i);
  assert.equal(encoderAdvisories({ tune: "auto", crf: 21 }).length, 0);
  assert.equal(encoderAdvisories({ tune: "none", crf: 24 }).length, 0);
});

test("extreme CRF values get an advisory in both directions", () => {
  assert.match(encoderAdvisories({ tune: "none", crf: 16 })[0].text, /very large/i);
  assert.match(encoderAdvisories({ tune: "none", crf: 28 })[0].text, /soft|banding|detail/i);
  const both = encoderAdvisories({ tune: "grain", crf: 16 });
  assert.equal(both.length, 2);
});

test("the CRF size guide is relative to the default, falls as CRF rises, and is honest that it is rough", () => {
  assert.equal(crfSizeRatio(CRF_DEFAULT), 1);
  assert.ok(Math.abs(crfSizeRatio(27) - 0.5) < 0.01, "about every +6 halves the size");
  assert.ok(Math.abs(crfSizeRatio(15) - 2) < 0.01, "and every -6 doubles it");
  for (let crf = 1; crf <= 51; crf += 1) assert.ok(crfSizeRatio(crf) < crfSizeRatio(crf - 1), `CRF ${crf} is smaller than ${crf - 1}`);
  assert.equal(crfSizeRatio(-9), crfSizeRatio(0), "out-of-range values clamp");
  assert.match(crfSizeHint(CRF_DEFAULT), /default/i);
  assert.match(crfSizeHint(22), /close to/i, "a one-step change is not presented as a big difference");
  assert.match(crfSizeHint(27), /50%/);
  assert.match(crfSizeHint(15), /2×/);
  for (const crf of [0, 15, 21, 27, 40, 51]) assert.match(crfSizeHint(crf), /rough/i, `CRF ${crf} says it is only a guide`);
});

test("the top CRF band no longer claims to be for short tests, which now follow the run's CRF", () => {
  assert.doesNotMatch(crfBand(40).summary, /preview|test/i);
  assert.match(crfBand(40).summary, /loss|small/i);
  assert.match(SHORT_TEST_HELP, /5-second/);
  assert.match(SHORT_TEST_HELP, /same (quality|settings|CRF)/i);
  assert.match(SHORT_TEST_HELP, /original/i);
});

const REQUIRED_HELP = ["libx265", "detectAnimation", "hdr10plus", "dolbyVision", "secondaryVideo", "dataStreams", "copyVerification", "qualityCheck", "minSsim",
  "youtubeBitrateProfile", "youtubeSizeTarget", "youtubeShrink", "youtubeQualityCheck", "youtubeSdrCodec"];

test("every setting that had only jargon now has a titled plain-language explanation", () => {
  assert.deepEqual([...SETTING_HELP_KEYS].sort(), [...REQUIRED_HELP].sort());
  for (const key of REQUIRED_HELP) {
    const help = settingHelp(key);
    assert.ok(help.title.length > 3, `${key} has a title`);
    assert.ok(help.text.length >= 60 && help.text.length <= 520, `${key} is a short paragraph, not a stub or an essay (${help.text.length})`);
    assert.doesNotMatch(help.text, /dhdr10|hdr10plus_tool|HDR10_PLUS|\binject\b|\bremux\b|\bffprobe\b/i, `${key} avoids internal tool names`);
  }
});

test("technical terms are explained where they are used", () => {
  assert.match(settingHelp("hdr10plus").text, /brightness/i);
  assert.match(settingHelp("hdr10plus").text, /scene/i);
  for (const word of ["Safe", "Skip", "HDR10"]) assert.match(settingHelp("dolbyVision").text, new RegExp(word));
  assert.match(settingHelp("dolbyVision").text, /Dolby Vision/);
  assert.match(settingHelp("secondaryVideo").text, /Encode/);
  assert.match(settingHelp("secondaryVideo").text, /Copy/);
  assert.match(settingHelp("copyVerification").text, /fingerprint/i);
  assert.match(settingHelp("copyVerification").text, /never re-?encoded|copied/i);
  for (const key of ["qualityCheck", "minSsim", "youtubeQualityCheck"]) {
    assert.match(settingHelp(key).text, /score/i, `${key} says what the score is`);
    assert.match(settingHelp(key).text, /SSIM/, `${key} names the measure once so it can be looked up`);
  }
  assert.match(settingHelp("libx265").text, /encoder/i);
  assert.match(settingHelp("libx265").text, /HEVC/);
  assert.match(settingHelp("minSsim").text, /grain/i, "the grainy-film caveat is kept");
  assert.match(settingHelp("qualityCheck").text, /Enforce/);
  assert.match(settingHelp("qualityCheck").text, /original/i);
});

test("the YouTube options say what each choice does to the file", () => {
  assert.match(settingHelp("youtubeBitrateProfile").text, /Balanced/);
  assert.match(settingHelp("youtubeBitrateProfile").text, /YouTube/);
  assert.match(settingHelp("youtubeSizeTarget").text, /not a strict/i);
  for (const word of ["Auto", "Off", "Aggressive"]) assert.match(settingHelp("youtubeShrink").text, new RegExp(word));
  assert.match(settingHelp("youtubeSdrCodec").text, /H\.264/);
  assert.match(settingHelp("youtubeSdrCodec").text, /HEVC/);
});

test("an unknown help key reads as a harmless generic note instead of throwing", () => {
  const help = settingHelp("not-a-setting");
  assert.ok(help.title && help.text);
});

const shared = JSON.parse(readFileSync(new URL("../src/lib/help-content.json", import.meta.url), "utf8"));

test("the GUI and the MCP server read one file, so what the app says and what a model is told cannot drift", () => {
  assert.equal(CRF_DEFAULT, shared.crf.default);
  assert.deepEqual(CRF_BAND_TABLE, shared.crf.bands);
  for (const key of SETTING_HELP_KEYS) assert.deepEqual(settingHelp(key), shared.settings[key], key);
  for (const tune of Object.keys(shared.tunes)) assert.deepEqual(tuneHelp(tune), shared.tunes[tune], tune);
  for (const preset of Object.keys(shared.presets)) assert.equal(presetHelp(preset), shared.presets[preset], preset);
  assert.equal(SHORT_TEST_HELP, shared.shortTest);
});

test("the CRF band table covers 0 to 51 once, in order, and marks exactly the default's band as recommended", () => {
  let previous = -1;
  for (const band of shared.crf.bands) {
    assert.ok(band.max > previous, "bands are in increasing order");
    previous = band.max;
  }
  assert.equal(previous, 51, "the last band reaches 51");
  assert.equal(shared.crf.bands.filter((band) => band.recommended).length, 1);
  assert.equal(crfBand(shared.crf.default).recommended, true);
});

test("the guide has five sections, each titled, summarised and made of short readable items", () => {
  assert.deepEqual([...GUIDE_SECTIONS].sort(), ["headless", "mcp", "modes", "safety", "workflow"]);
  for (const key of GUIDE_SECTIONS) {
    const section = guideSection(key);
    assert.ok(section.title && section.summary, key);
    assert.ok(section.items.length >= 3, `${key} has several items`);
    for (const item of section.items) {
      assert.ok(item.title.length >= 2, `${key}: ${item.title}`);
      assert.ok(item.text.length >= 40 && item.text.length <= 620, `${key}/${item.title}: ${item.text.length} characters`);
    }
  }
  assert.equal(guideSection("not-a-section").items.length, 0, "an unknown section is empty, not an error");
});

test("the headless guide covers sign-in, read-only, folders, agents, HTTPS and the no-password rule", () => {
  const text = guideSection("headless").items.map((item) => `${item.title} ${item.text}`).join("\n");
  for (const needle of ["serve", "127.0.0.1:8437", "login link", "Bearer", "--read-only", "--root", "/mcp", "reverse proxy", "--allow-remote", "passwordless sudo", "--no-default-features"]) {
    assert.ok(text.includes(needle), `mentions ${needle}`);
  }
  assert.match(text, /no password prompt/i);
});

test("the agent workflow starts read-only and never skips confirmation", () => {
  const workflow = guideSection("workflow").items.map((item) => item.text).join(" ");
  assert.match(workflow, /inspect_environment/);
  assert.match(workflow, /check/);
  assert.match(workflow, /confirm_full_rebuild/);
  const mcp = guideSection("mcp").items.map((item) => item.text).join(" ");
  assert.match(mcp, /explain_settings/);
  assert.match(mcp, /app_guide/);
  assert.match(mcp, /already active/);
});
