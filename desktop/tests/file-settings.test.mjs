// ./desktop/tests/file-settings.test.mjs
// Characterization tests for the per-file run settings module extracted from the route (helper slice):
// YouTube estimates and resource forecasts, tune overrides, effective settings per file, default and
// chosen streams, the run-settings editor, current-run overrides and HDR10+ loss approvals.
// Run: pnpm test:file-settings

import assert from "node:assert/strict";
import test from "node:test";
import { createFileSettings } from "../src/lib/file-settings.ts";

const track = (index, type, over = {}) => ({ index, type, width: 0, height: 0, default: false, attached_picture: false, bit_depth: null, ...over });
const profile = (tracks = [], over = {}) => ({ duration_seconds: 3600, audio_count: 2, content: {}, tracks, ...over });
const entry = (path, over = {}) => ({ path, name: path.split("/").at(-1), size_bytes: 4_000_000_000, profile: profile(), ...over });
const settings = (over = {}) => ({ content: { tune: "auto" }, streams: { secondary_video: "encode", data_streams: "preserve" }, ...over });

function harness({ hostOver = {}, reviewed = () => undefined } = {}) {
  const log = [];
  const state = {
    youtubeMode: "upscale", youtubeResolution: "1440p", youtubeQualityMode: "balanced", youtubeSourceMultiplier: 2, youtubeHdr10plusLossPaths: new Set(),
    mode: "check", fileTuneOverrides: {}, fileStreamOverrides: {}, fileSettingsOverrides: {}, runSettingsEditorOpen: false, runSettingsEditorPaths: [],
    settings: settings(), queueMovieReviews: {}, queueSources: [], activeRoot: "/lib", selectedRunEntries: [], workflowBusy: false, ...hostOver,
  };
  const host = state;
  host.setNotice = (message, kind = "info") => log.push([kind, message]);
  const file = createFileSettings({ host, actions: { reviewedSeriesForPath: reviewed } });
  return { file, host, log };
}

const estimateFor = (over = {}) => ({ recommended_video_bitrate_kbps: 16000, video_bitrate_kbps: 16000, estimated_output_bytes: 1, ...over });
const ytEntry = (over = {}) => entry("/lib/a.mkv", { youtube_estimates: { "1440p": { balanced: estimateFor(), recommended: estimateFor({ video_bitrate_kbps: 20000 }) } }, ...over });

test("youtubeEstimateFor returns nothing in optimize mode and the stored estimate for other qualities", () => {
  const optimize = harness({ hostOver: { youtubeMode: "optimize" } });
  assert.equal(optimize.file.youtubeEstimateFor(ytEntry()), undefined);
  const recommended = harness({ hostOver: { youtubeQualityMode: "recommended" } });
  assert.equal(recommended.file.youtubeEstimateFor(ytEntry()).video_bitrate_kbps, 20000);
  const none = harness();
  assert.equal(none.file.youtubeEstimateFor(entry("/lib/b.mkv")), undefined);
  assert.equal(none.file.youtubeEstimateFor(ytEntry({ profile: null })).video_bitrate_kbps, 16000);
  assert.equal(none.file.youtubeEstimateFor(ytEntry({ profile: profile([], { duration_seconds: 0 }) })).video_bitrate_kbps, 16000);
});

test("youtubeEstimateFor aligns the balanced forecast with the source bitrate and caps it at the recommendation", () => {
  const h = harness();
  const low = ytEntry({ size_bytes: 3_600_000_000, profile: profile([], { duration_seconds: 3600, audio_count: 1 }) });
  const result = h.file.youtubeEstimateFor(low);
  const sourceKbps = 3_600_000_000 * 8 / 3600 / 1000;
  const expectedVideo = Math.min(16000, Math.max(250, Math.round(sourceKbps * 2 - 384)));
  assert.equal(result.video_bitrate_kbps, expectedVideo);
  assert.equal(result.estimated_output_bytes, Math.max(0, Math.round(3600 * (expectedVideo + 384) * 1000 / 8 * 1.12)));
  const huge = ytEntry({ size_bytes: 40_000_000_000 });
  assert.equal(h.file.youtubeEstimateFor(huge).video_bitrate_kbps, 16000);
  const tiny = ytEntry({ size_bytes: 1000 });
  assert.equal(h.file.youtubeEstimateFor(tiny).video_bitrate_kbps, 250);
});

test("resourceForecastFor uses the YouTube forecast only in YouTube mode, else the plain one", () => {
  const item = entry("/lib/a.mkv", { resource_forecast: { level: "plain" }, youtube_resource_forecasts: { "1440p": { level: "yt" } } });
  assert.equal(harness().file.resourceForecastFor(item).level, "plain");
  assert.equal(harness({ hostOver: { mode: "youtube" } }).file.resourceForecastFor(item).level, "yt");
  assert.equal(harness({ hostOver: { mode: "youtube", youtubeResolution: "4k" } }).file.resourceForecastFor(item).level, "plain");
});

test("setFileTuneOverride stores animation or none and removes anything else", () => {
  const h = harness();
  h.file.setFileTuneOverride("/a", "animation");
  h.file.setFileTuneOverride("/b", "none");
  assert.deepEqual(h.host.fileTuneOverrides, { "/a": "animation", "/b": "none" });
  h.file.setFileTuneOverride("/a", "auto");
  assert.deepEqual(h.host.fileTuneOverrides, { "/b": "none" });
});

test("settingsForEntry prefers the active source's draft and other sources' own scan settings", () => {
  const active = { root: "/lib", scanResult: { settings: { id: "scan-active" }, entries: [{ path: "/lib/a.mkv" }] } };
  const other = { root: "/other", scanResult: { settings: { id: "scan-other" }, entries: [{ path: "/other/b.mkv" }] } };
  const h = harness({ hostOver: { queueSources: [active, other], settings: { id: "draft" } } });
  assert.equal(h.file.settingsForEntry(entry("/lib/a.mkv")).id, "draft");
  assert.equal(h.file.settingsForEntry(entry("/other/b.mkv")).id, "scan-other");
  assert.equal(h.file.settingsForEntry(entry("/nowhere.mkv")).id, "draft");
  h.host.settings = null;
  assert.equal(h.file.settingsForEntry(entry("/lib/a.mkv")).id, "scan-active");
  const noScan = harness({ hostOver: { queueSources: [{ root: "/other", scanResult: null }], settings: { id: "draft" } } });
  assert.equal(noScan.file.settingsForEntry(entry("/x.mkv")).id, "draft");
  const emptyOther = harness({ hostOver: { queueSources: [{ root: "/other", scanResult: { settings: null, entries: [{ path: "/other/b.mkv" }] } }], settings: { id: "draft" } } });
  assert.equal(emptyOther.file.settingsForEntry(entry("/other/b.mkv")).id, "draft");
});

test("effectiveContentTuneForEntry orders override, animation hints, then the global tune", () => {
  const h = harness();
  const item = entry("/a.mkv");
  assert.equal(h.file.effectiveContentTuneForEntry(item, undefined, settings()), "none");
  assert.equal(h.file.effectiveContentTuneForEntry(item, undefined, settings({ content: { tune: "grain" } })), "grain");
  assert.equal(h.file.effectiveContentTuneForEntry(item, undefined, settings({ content: { tune: "animation" } })), "animation");
  assert.equal(h.file.effectiveContentTuneForEntry(item, undefined, undefined), "none");
  assert.equal(h.file.effectiveContentTuneForEntry(item, { genresText: "Anime" }, settings({ content: { tune: "grain" } })), "animation");
  assert.equal(h.file.effectiveContentTuneForEntry(entry("/b.mkv", { profile: profile([], { content: { animation_detected: true } }) }), undefined, settings()), "animation");
  h.host.fileTuneOverrides = { "/a.mkv": "none" };
  assert.equal(h.file.effectiveContentTuneForEntry(item, { genresText: "Anime" }, settings()), "none");
});

test("fileTuneOverridesForBatch keeps explicit overrides and adds detected animation", () => {
  const detected = entry("/lib/detected.mkv", { profile: profile([], { content: { animation_detected: true } }) });
  const reviewedAnime = entry("/lib/reviewed.mkv");
  const plain = entry("/lib/plain.mkv");
  const explicit = entry("/lib/explicit.mkv", { profile: profile([], { content: { animation_detected: true } }) });
  const h = harness({
    hostOver: { fileTuneOverrides: { "/lib/explicit.mkv": "none", "/lib/not-in-batch.mkv": "animation" }, queueMovieReviews: { "/lib/reviewed.mkv": { genresText: "Animation" } } },
    reviewed: (path) => (path === "/lib/plain.mkv" ? { genresText: "Drama" } : undefined),
  });
  assert.deepEqual(h.file.fileTuneOverridesForBatch([detected, reviewedAnime, plain, explicit], settings()), { "/lib/explicit.mkv": "none", "/lib/detected.mkv": "animation", "/lib/reviewed.mkv": "animation" });
});

const tracks = () => [
  track(0, "video", { width: 1920, height: 1080, default: true }), track(1, "video", { width: 3840, height: 2160, bit_depth: 10 }), track(2, "video", { width: 600, height: 600, attached_picture: true }),
  track(3, "audio"), track(4, "subtitle"), track(5, "attachment"), track(6, "data"),
];

test("defaultStreamIndices keeps every real video unless secondary video is dropped, plus audio, subtitle, attachment and preserved data", () => {
  const item = entry("/lib/a.mkv", { profile: profile(tracks()) });
  const h = harness();
  assert.deepEqual(h.file.defaultStreamIndices(item), [0, 1, 3, 4, 5, 6]);
  h.host.settings = settings({ streams: { secondary_video: "drop", data_streams: "drop" } });
  assert.deepEqual(h.file.defaultStreamIndices(item), [1, 3, 4, 5], "the largest video wins and data is dropped");
  h.host.settings = null;
  assert.deepEqual(h.file.defaultStreamIndices(item), [0, 1, 3, 4, 5, 6], "no settings means preserve");
  assert.deepEqual(h.file.defaultStreamIndices({ profile: null }), []);
});

test("selectedStreamIndices prefers the saved override and selectedTrackCount counts it", () => {
  const item = entry("/lib/a.mkv", { profile: profile(tracks()) });
  const h = harness();
  assert.deepEqual(h.file.selectedStreamIndices(item), [0, 1, 3, 4, 5, 6]);
  h.host.fileStreamOverrides = { "/lib/a.mkv": [1, 3] };
  assert.deepEqual([h.file.selectedStreamIndices(item), h.file.selectedTrackCount(item)], [[1, 3], 2]);
});

test("setFileStreamSelected toggles tracks, protects the last real video and drops overrides equal to the defaults", () => {
  const item = entry("/lib/a.mkv", { profile: profile(tracks()) });
  const h = harness();
  h.file.setFileStreamSelected(item, 4, false);
  assert.deepEqual(h.host.fileStreamOverrides["/lib/a.mkv"], [0, 1, 3, 5, 6]);
  h.file.setFileStreamSelected(item, 4, true);
  assert.deepEqual(h.host.fileStreamOverrides, {}, "back to the defaults removes the override");
  h.host.fileStreamOverrides = { "/lib/a.mkv": [1, 3, 4] };
  h.file.setFileStreamSelected(item, 1, false);
  assert.deepEqual(h.log, [["error", "Keep at least one real video track selected for a rebuild."]]);
  assert.deepEqual(h.host.fileStreamOverrides["/lib/a.mkv"], [1, 3, 4]);
  h.file.setFileStreamSelected(item, 0, true);
  assert.deepEqual(h.host.fileStreamOverrides["/lib/a.mkv"], [0, 1, 3, 4]);
});

test("streamIndicesForMode handles default, all, primary and no-data", () => {
  const item = entry("/lib/a.mkv", { profile: profile(tracks()) });
  const h = harness();
  assert.deepEqual(h.file.streamIndicesForMode(item, "default"), [0, 1, 3, 4, 5, 6]);
  assert.deepEqual(h.file.streamIndicesForMode(item, "all"), [0, 1, 2, 3, 4, 5, 6]);
  assert.deepEqual(h.file.streamIndicesForMode(item, "primary"), [1, 3, 4, 5, 6]);
  assert.deepEqual(h.file.streamIndicesForMode(item, "no-data"), [1, 3, 4, 5]);
});

test("runEntriesForFolder keeps selected run entries inside a folder", () => {
  const h = harness({ hostOver: { selectedRunEntries: [entry("/lib/A/x.mkv"), entry("/lib/A/sub/y.mkv"), entry("/lib/AB/z.mkv"), entry("/lib/A")] } });
  assert.deepEqual(h.file.runEntriesForFolder("/lib/A").map((item) => item.path), ["/lib/A/x.mkv", "/lib/A/sub/y.mkv", "/lib/A"]);
});

test("runOverrideSummary lists the current-run choices", () => {
  const item = entry("/lib/a.mkv");
  const h = harness();
  assert.equal(h.file.runOverrideSummary(item), "Uses reviewed run settings");
  h.host.fileSettingsOverrides = { "/lib/a.mkv": {} };
  assert.equal(h.file.runOverrideSummary(item), "Uses reviewed run settings");
  h.host.fileSettingsOverrides = { "/lib/a.mkv": { crf: 20, preset: "slow", tune: "grain", promote_8bit_to_10bit: true, preserve_hdr10plus: false, dolby_vision: "hdr10-fallback", stream_indices: [0, 1] } };
  assert.equal(h.file.runOverrideSummary(item), "Current run: CRF 20 · slow · film grain · 10-bit promotion · no HDR10+ · HDR10 fallback · 2 tracks");
  h.host.fileSettingsOverrides = { "/lib/a.mkv": { tune: "animation", promote_8bit_to_10bit: false, preserve_hdr10plus: true, dolby_vision: "skip" } };
  assert.equal(h.file.runOverrideSummary(item), "Current run: animation · 8-bit output · HDR10+ · Dolby skip");
});

test("openRunSettingsEditor selects the matching ready files, or explains when none match, and ignores a busy run", () => {
  const files = [entry("/lib/a.mkv"), entry("/lib/b.mkv")];
  const h = harness({ hostOver: { selectedRunEntries: files } });
  h.file.openRunSettingsEditor(["/lib/b.mkv", "/missing.mkv"]);
  assert.deepEqual([h.host.runSettingsEditorOpen, h.host.runSettingsEditorPaths], [true, ["/lib/b.mkv"]]);
  const none = harness({ hostOver: { selectedRunEntries: files } });
  none.file.openRunSettingsEditor(["/missing.mkv"]);
  assert.deepEqual([none.host.runSettingsEditorOpen, none.log], [false, [["error", "Select or scan at least one ready media file before adjusting this run."]]]);
  const busy = harness({ hostOver: { selectedRunEntries: files, workflowBusy: true } });
  busy.file.openRunSettingsEditor(["/lib/a.mkv"]);
  assert.deepEqual([busy.host.runSettingsEditorOpen, busy.log.length], [false, 0]);
});

test("applyRunSettingsOverride merges choices, applies a stream mode and prunes empty overrides", () => {
  const item = entry("/lib/a.mkv", { profile: profile(tracks()) });
  const h = harness({ hostOver: { selectedRunEntries: [item], fileSettingsOverrides: { "/lib/a.mkv": { crf: 18 }, "/lib/gone.mkv": { crf: 1 } } } });
  h.file.applyRunSettingsOverride(["/lib/a.mkv"], { preset: "slow" }, "primary");
  assert.deepEqual(h.host.fileSettingsOverrides["/lib/a.mkv"], { crf: 18, preset: "slow", stream_indices: [1, 3, 4, 5, 6] });
  assert.deepEqual(h.log.at(-1), ["success", "Applied current-run choices to 1 file. Saved defaults were not changed."]);
  h.file.applyRunSettingsOverride(["/lib/a.mkv", "/lib/other.mkv"], { dolby_vision: "skip" }, "keep");
  assert.equal(h.host.fileSettingsOverrides["/lib/a.mkv"].stream_indices.length, 5, "keep leaves the streams alone");
  assert.deepEqual(h.host.fileSettingsOverrides["/lib/other.mkv"], { dolby_vision: "skip" });
  assert.match(h.log.at(-1)[1], /to 2 files/);
  h.file.applyRunSettingsOverride(["/lib/new.mkv"], {}, "exact");
  assert.equal("/lib/new.mkv" in h.host.fileSettingsOverrides, false);
});

test("clearRunSettingsOverride removes the listed files' choices and says so", () => {
  const h = harness({ hostOver: { fileSettingsOverrides: { "/a": { crf: 1 }, "/b": { crf: 2 } } } });
  h.file.clearRunSettingsOverride(["/a"]);
  assert.deepEqual([h.host.fileSettingsOverrides, h.log], [{ "/b": { crf: 2 } }, [["info", "Cleared current-run choices for 1 file."]]]);
  h.file.clearRunSettingsOverride(["/b", "/c"]);
  assert.deepEqual(h.log.at(-1), ["info", "Cleared current-run choices for 2 files."]);
});

test("HDR10+ loss approvals add and remove paths, replace the set and cap at 512 files", () => {
  const h = harness();
  assert.equal(h.file.youtubeHdr10plusLossAccepted("/a"), false);
  const before = h.host.youtubeHdr10plusLossPaths;
  h.file.setYoutubeHdr10plusLossAccepted("/a", true);
  assert.deepEqual([h.file.youtubeHdr10plusLossAccepted("/a"), h.host.youtubeHdr10plusLossPaths === before], [true, false]);
  h.file.setYoutubeHdr10plusLossAccepted("/a", false);
  assert.equal(h.file.youtubeHdr10plusLossAccepted("/a"), false);
  h.host.youtubeHdr10plusLossPaths = new Set(Array.from({ length: 512 }, (_, index) => `/p${index}`));
  h.file.setYoutubeHdr10plusLossAccepted("/new", true);
  assert.equal(h.host.youtubeHdr10plusLossPaths.has("/new"), false);
  assert.deepEqual(h.log.at(-1), ["error", "Per-run HDR10+ loss approval is limited to 512 files. Split this library into smaller export batches."]);
  h.file.setYoutubeHdr10plusLossAccepted("/p3", true);
  assert.equal(h.log.length, 1, "re-approving an existing path is allowed at the cap");
});
