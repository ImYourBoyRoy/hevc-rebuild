// ./desktop/tests/credentials-controller.test.mjs
// Characterization tests for the metadata-provider credentials controller extracted from the route
// (credentials slice): TMDB token and TheTVDB key/PIN status, save, reveal, show/hide and remove.
// The native bridge is an injected fake and the route's drafts are a plain host object.
// Run: pnpm test:credentials-controller

import assert from "node:assert/strict";
import test from "node:test";
import { createCredentialsController } from "../src/lib/credentials-controller.ts";

function harness({ desktop = true, invokeImpl = {}, hostOver = {} } = {}) {
  const log = [];
  const invokes = [];
  const host = {
    tmdbCredentialStatus: { configured: false, secureStoreAvailable: true, message: null }, tmdbTokenDraft: "", tmdbTokenVisible: false, tmdbCredentialBusy: false,
    tvdbCredentialStatus: { configured: false, subscriberPinConfigured: false, secureStoreAvailable: true, message: null },
    tvdbApiKeyDraft: "", tvdbSubscriberPinDraft: "", tvdbCredentialsVisible: false, tvdbAccessConfirmed: false, tvdbCredentialBusy: false,
    setNotice: (message, kind = "info") => log.push([kind, message]),
    ...hostOver,
  };
  const invoke = async (command, args) => {
    invokes.push({ command, args });
    const impl = invokeImpl[command];
    if (impl === undefined) throw new Error(`unexpected ${command}`);
    return typeof impl === "function" ? impl(args) : impl;
  };
  return { controller: createCredentialsController({ invoke, isDesktop: () => desktop, host }), host, log, invokes };
}

const boom = (message) => () => { throw new Error(message); };

test("refreshTmdbCredentialStatus reads the keyring status on desktop", async () => {
  const status = { configured: true, secureStoreAvailable: true, message: null };
  const h = harness({ invokeImpl: { tmdb_credential_status: status } });
  await h.controller.refreshTmdbCredentialStatus();
  assert.deepEqual(h.host.tmdbCredentialStatus, status);
});

test("refreshTmdbCredentialStatus explains the browser preview and reports a bridge failure", async () => {
  const preview = harness({ desktop: false });
  await preview.controller.refreshTmdbCredentialStatus();
  assert.deepEqual([preview.host.tmdbCredentialStatus.configured, preview.host.tmdbCredentialStatus.secureStoreAvailable], [false, false]);
  assert.match(preview.host.tmdbCredentialStatus.message, /requires the installed desktop app and its operating-system keyring/);
  assert.equal(preview.invokes.length, 0);
  const failed = harness({ invokeImpl: { tmdb_credential_status: boom("no keyring") } });
  await failed.controller.refreshTmdbCredentialStatus();
  assert.deepEqual(failed.host.tmdbCredentialStatus, { configured: false, secureStoreAvailable: false, message: "Error: no keyring" });
});

test("refreshTvdbCredentialStatus reads, explains the preview and reports failure", async () => {
  const status = { configured: true, subscriberPinConfigured: true, secureStoreAvailable: true, message: null };
  const ok = harness({ invokeImpl: { tvdb_credential_status: status } });
  await ok.controller.refreshTvdbCredentialStatus();
  assert.deepEqual(ok.host.tvdbCredentialStatus, status);
  const preview = harness({ desktop: false });
  await preview.controller.refreshTvdbCredentialStatus();
  assert.deepEqual([preview.host.tvdbCredentialStatus.configured, preview.host.tvdbCredentialStatus.subscriberPinConfigured, preview.host.tvdbCredentialStatus.secureStoreAvailable], [false, false, false]);
  assert.match(preview.host.tvdbCredentialStatus.message, /Saving provider credentials requires the installed desktop app/);
  const failed = harness({ invokeImpl: { tvdb_credential_status: boom("denied") } });
  await failed.controller.refreshTvdbCredentialStatus();
  assert.deepEqual(failed.host.tvdbCredentialStatus, { configured: false, subscriberPinConfigured: false, secureStoreAvailable: false, message: "Error: denied" });
});

test("saveTmdbCredential trims and saves the token, shows it and refreshes the status", async () => {
  const h = harness({ invokeImpl: { save_tmdb_read_token: null, tmdb_credential_status: { configured: true, secureStoreAvailable: true, message: null } }, hostOver: { tmdbTokenDraft: "  tok-123  " } });
  const pending = h.controller.saveTmdbCredential();
  assert.equal(h.host.tmdbCredentialBusy, true);
  await pending;
  assert.deepEqual(h.invokes.map((call) => call.command), ["save_tmdb_read_token", "tmdb_credential_status"]);
  assert.deepEqual(h.invokes[0].args, { token: "tok-123" });
  assert.deepEqual([h.host.tmdbTokenVisible, h.host.tmdbCredentialBusy, h.host.tmdbCredentialStatus.configured], [true, false, true]);
  assert.deepEqual(h.log, [["success", "TMDB token saved in the operating-system keyring."]]);
});

test("saveTmdbCredential refuses an empty token, off desktop or while busy, and reports a failure", async () => {
  const empty = harness({ hostOver: { tmdbTokenDraft: "   " } });
  await empty.controller.saveTmdbCredential();
  assert.deepEqual(empty.log, [["error", "Paste the TMDB API Read Access Token before saving."]]);
  assert.equal(empty.invokes.length, 0);
  for (const [options, hostOver] of [[{ desktop: false }, { tmdbTokenDraft: "t" }], [{}, { tmdbTokenDraft: "t", tmdbCredentialBusy: true }]]) {
    const h = harness({ ...options, hostOver });
    await h.controller.saveTmdbCredential();
    assert.deepEqual([h.invokes.length, h.log.length], [0, 0]);
  }
  const failed = harness({ invokeImpl: { save_tmdb_read_token: boom("keyring locked") }, hostOver: { tmdbTokenDraft: "t" } });
  await failed.controller.saveTmdbCredential();
  assert.deepEqual(failed.log, [["error", "Could not save the TMDB token: Error: keyring locked"]]);
  assert.equal(failed.host.tmdbCredentialBusy, false);
});

test("revealTmdbCredential reads the saved token only when one is configured", async () => {
  const h = harness({ invokeImpl: { reveal_tmdb_read_token: "saved-token" }, hostOver: { tmdbCredentialStatus: { configured: true, secureStoreAvailable: true, message: null } } });
  await h.controller.revealTmdbCredential();
  assert.deepEqual([h.host.tmdbTokenDraft, h.host.tmdbTokenVisible, h.host.tmdbCredentialBusy], ["saved-token", true, false]);
  for (const [options, hostOver] of [[{}, {}], [{ desktop: false }, { tmdbCredentialStatus: { configured: true } }], [{}, { tmdbCredentialStatus: { configured: true }, tmdbCredentialBusy: true }]]) {
    const blocked = harness({ ...options, hostOver });
    await blocked.controller.revealTmdbCredential();
    assert.equal(blocked.invokes.length, 0);
  }
  const failed = harness({ invokeImpl: { reveal_tmdb_read_token: boom("denied") }, hostOver: { tmdbCredentialStatus: { configured: true } } });
  await failed.controller.revealTmdbCredential();
  assert.deepEqual(failed.log, [["error", "Could not read the saved TMDB token: Error: denied"]]);
  assert.equal(failed.host.tmdbCredentialBusy, false);
});

test("toggleTmdbCredentialVisibility reveals an empty draft and otherwise flips visibility", async () => {
  const h = harness({ invokeImpl: { reveal_tmdb_read_token: "saved" }, hostOver: { tmdbCredentialStatus: { configured: true } } });
  h.controller.toggleTmdbCredentialVisibility();
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual([h.host.tmdbTokenDraft, h.host.tmdbTokenVisible], ["saved", true]);
  h.controller.toggleTmdbCredentialVisibility();
  assert.equal(h.host.tmdbTokenVisible, false);
  h.controller.toggleTmdbCredentialVisibility();
  assert.equal(h.host.tmdbTokenVisible, true);
});

test("removeTmdbCredential clears the draft, shows the field and refreshes the status", async () => {
  const h = harness({ invokeImpl: { remove_tmdb_read_token: null, tmdb_credential_status: { configured: false, secureStoreAvailable: true, message: null } }, hostOver: { tmdbTokenDraft: "t", tmdbTokenVisible: false } });
  await h.controller.removeTmdbCredential();
  assert.deepEqual([h.host.tmdbTokenDraft, h.host.tmdbTokenVisible, h.host.tmdbCredentialBusy, h.host.tmdbCredentialStatus.configured], ["", true, false, false]);
  assert.deepEqual(h.log, [["success", "Saved TMDB token removed from the operating-system keyring."]]);
  const failed = harness({ invokeImpl: { remove_tmdb_read_token: boom("busy") }, hostOver: { tmdbTokenDraft: "keep" } });
  await failed.controller.removeTmdbCredential();
  assert.deepEqual([failed.host.tmdbTokenDraft, failed.log], ["keep", [["error", "Could not remove the TMDB token: Error: busy"]]]);
  const off = harness({ desktop: false });
  await off.controller.removeTmdbCredential();
  assert.equal(off.invokes.length, 0);
});

test("saveTvdbCredential needs a key and access confirmation, then saves trimmed values", async () => {
  const none = harness({ hostOver: { tvdbApiKeyDraft: " " } });
  await none.controller.saveTvdbCredential();
  assert.deepEqual(none.log, [["error", "Enter a TheTVDB project API key before saving."]]);
  const unconfirmed = harness({ hostOver: { tvdbApiKeyDraft: "key" } });
  await unconfirmed.controller.saveTvdbCredential();
  assert.deepEqual(unconfirmed.log, [["error", "Confirm that this project has valid TheTVDB API access before saving."]]);
  const h = harness({ invokeImpl: { save_tvdb_credentials: null, tvdb_credential_status: { configured: true, subscriberPinConfigured: false, secureStoreAvailable: true, message: null } }, hostOver: { tvdbApiKeyDraft: " key ", tvdbSubscriberPinDraft: "  ", tvdbAccessConfirmed: true } });
  await h.controller.saveTvdbCredential();
  assert.deepEqual(h.invokes[0], { command: "save_tvdb_credentials", args: { apiKey: "key", subscriberPin: null, accessConfirmed: true } });
  assert.deepEqual([h.host.tvdbCredentialsVisible, h.host.tvdbAccessConfirmed, h.host.tvdbCredentialBusy, h.host.tvdbCredentialStatus.configured], [true, false, false, true]);
  assert.deepEqual(h.log, [["success", "TheTVDB credentials were saved in the operating-system keyring. Their validity will be checked on the next search."]]);
  const withPin = harness({ invokeImpl: { save_tvdb_credentials: null, tvdb_credential_status: {} }, hostOver: { tvdbApiKeyDraft: "k", tvdbSubscriberPinDraft: " 1234 ", tvdbAccessConfirmed: true } });
  await withPin.controller.saveTvdbCredential();
  assert.equal(withPin.invokes[0].args.subscriberPin, "1234");
});

test("saveTvdbCredential is blocked off desktop or while busy and reports a failure", async () => {
  for (const [options, hostOver] of [[{ desktop: false }, { tvdbApiKeyDraft: "k", tvdbAccessConfirmed: true }], [{}, { tvdbApiKeyDraft: "k", tvdbAccessConfirmed: true, tvdbCredentialBusy: true }]]) {
    const h = harness({ ...options, hostOver });
    await h.controller.saveTvdbCredential();
    assert.deepEqual([h.invokes.length, h.log.length], [0, 0]);
  }
  const failed = harness({ invokeImpl: { save_tvdb_credentials: boom("locked") }, hostOver: { tvdbApiKeyDraft: "k", tvdbAccessConfirmed: true } });
  await failed.controller.saveTvdbCredential();
  assert.deepEqual(failed.log, [["error", "Could not save TheTVDB credentials: Error: locked"]]);
  assert.deepEqual([failed.host.tvdbCredentialBusy, failed.host.tvdbAccessConfirmed], [false, true]);
});

test("revealTvdbCredentials fills the key and PIN and tolerates a missing PIN", async () => {
  const configured = { tvdbCredentialStatus: { configured: true, subscriberPinConfigured: true, secureStoreAvailable: true, message: null } };
  const h = harness({ invokeImpl: { reveal_tvdb_credentials: { apiKey: "saved-key", subscriberPin: "9876" } }, hostOver: configured });
  await h.controller.revealTvdbCredentials();
  assert.deepEqual([h.host.tvdbApiKeyDraft, h.host.tvdbSubscriberPinDraft, h.host.tvdbCredentialsVisible, h.host.tvdbCredentialBusy], ["saved-key", "9876", true, false]);
  const noPin = harness({ invokeImpl: { reveal_tvdb_credentials: { apiKey: "k", subscriberPin: null } }, hostOver: configured });
  await noPin.controller.revealTvdbCredentials();
  assert.equal(noPin.host.tvdbSubscriberPinDraft, "");
  const blocked = harness();
  await blocked.controller.revealTvdbCredentials();
  assert.equal(blocked.invokes.length, 0);
  const failed = harness({ invokeImpl: { reveal_tvdb_credentials: boom("denied") }, hostOver: configured });
  await failed.controller.revealTvdbCredentials();
  assert.deepEqual(failed.log, [["error", "Could not read the saved TheTVDB credentials: Error: denied"]]);
});

test("toggleTvdbCredentialVisibility reveals saved credentials when the drafts are empty, else flips", async () => {
  const h = harness({ invokeImpl: { reveal_tvdb_credentials: { apiKey: "k", subscriberPin: null } }, hostOver: { tvdbCredentialStatus: { configured: true, subscriberPinConfigured: false, secureStoreAvailable: true, message: null } } });
  h.controller.toggleTvdbCredentialVisibility();
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual([h.host.tvdbApiKeyDraft, h.host.tvdbCredentialsVisible], ["k", true]);
  h.controller.toggleTvdbCredentialVisibility();
  assert.equal(h.host.tvdbCredentialsVisible, false);
  const unconfigured = harness({ hostOver: { tvdbCredentialsVisible: false } });
  unconfigured.controller.toggleTvdbCredentialVisibility();
  assert.deepEqual([unconfigured.host.tvdbCredentialsVisible, unconfigured.invokes.length], [true, 0]);
});

test("removeTvdbCredential clears both drafts and the confirmation and refreshes the status", async () => {
  const h = harness({
    invokeImpl: { remove_tvdb_credentials: null, tvdb_credential_status: { configured: false, subscriberPinConfigured: false, secureStoreAvailable: true, message: null } },
    hostOver: { tvdbApiKeyDraft: "k", tvdbSubscriberPinDraft: "1", tvdbAccessConfirmed: true, tvdbCredentialsVisible: false },
  });
  await h.controller.removeTvdbCredential();
  assert.deepEqual([h.host.tvdbApiKeyDraft, h.host.tvdbSubscriberPinDraft, h.host.tvdbAccessConfirmed, h.host.tvdbCredentialsVisible, h.host.tvdbCredentialBusy], ["", "", false, true, false]);
  assert.deepEqual(h.log, [["success", "TheTVDB credentials were removed from the operating-system keyring."]]);
  const failed = harness({ invokeImpl: { remove_tvdb_credentials: boom("busy") }, hostOver: { tvdbApiKeyDraft: "keep", tvdbAccessConfirmed: true } });
  await failed.controller.removeTvdbCredential();
  assert.deepEqual([failed.host.tvdbApiKeyDraft, failed.host.tvdbAccessConfirmed, failed.log], ["keep", true, [["error", "Could not remove TheTVDB credentials: Error: busy"]]]);
  const off = harness({ desktop: false });
  await off.controller.removeTvdbCredential();
  assert.equal(off.invokes.length, 0);
});
