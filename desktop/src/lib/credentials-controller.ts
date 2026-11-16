// ./desktop/src/lib/credentials-controller.ts
// Metadata-provider credentials (TMDB read token, TheTVDB project key and PIN): status, save,
// reveal, show/hide and remove, extracted from the route (credentials slice). The route's drafts and
// flags are reached through a typed `CredentialsHost`; the native bridge is injected so every branch
// is unit-tested in node (pnpm test:credentials-controller). The code is the route's code moved
// verbatim with `h.` prefixes. Secrets only ever pass through the keyring commands.

import type { TmdbCredentialStatus, TvdbCredentialStatus } from './ui-contracts.js';

export type TvdbCredentials = { apiKey: string; subscriberPin: string | null };

export interface CredentialsHost {
  tmdbCredentialStatus: TmdbCredentialStatus;
  tmdbTokenDraft: string;
  tmdbTokenVisible: boolean;
  tmdbCredentialBusy: boolean;
  tvdbCredentialStatus: TvdbCredentialStatus;
  tvdbApiKeyDraft: string;
  tvdbSubscriberPinDraft: string;
  tvdbCredentialsVisible: boolean;
  tvdbAccessConfirmed: boolean;
  tvdbCredentialBusy: boolean;
  setNotice(message: string, kind?: "info" | "success" | "error"): void;
}

export interface CredentialsDeps {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  isDesktop: () => boolean;
  host: CredentialsHost;
}

export function createCredentialsController(deps: CredentialsDeps) {
  const { invoke, isDesktop, host: h } = deps;

  async function refreshTmdbCredentialStatus() {
    if (!isDesktop()) {
      h.tmdbCredentialStatus = { configured: false, secureStoreAvailable: false, message: "Saving API tokens requires the installed desktop app and its operating-system keyring. This browser preview does not store credentials." };
      return;
    }
    try {
      h.tmdbCredentialStatus = await invoke<TmdbCredentialStatus>("tmdb_credential_status");
    } catch (error) {
      h.tmdbCredentialStatus = { configured: false, secureStoreAvailable: false, message: String(error) };
    }
  }

  async function refreshTvdbCredentialStatus() {
    if (!isDesktop()) {
      h.tvdbCredentialStatus = {
        configured: false,
        subscriberPinConfigured: false,
        secureStoreAvailable: false,
        message: "Saving provider credentials requires the installed desktop app and its operating-system keyring. This browser preview does not store them.",
      };
      return;
    }
    try {
      h.tvdbCredentialStatus = await invoke<TvdbCredentialStatus>("tvdb_credential_status");
    } catch (error) {
      h.tvdbCredentialStatus = { configured: false, subscriberPinConfigured: false, secureStoreAvailable: false, message: String(error) };
    }
  }

  async function saveTmdbCredential() {
    if (!isDesktop() || h.tmdbCredentialBusy) return;
    if (!h.tmdbTokenDraft.trim()) {
      h.setNotice("Paste the TMDB API Read Access Token before saving.", "error");
      return;
    }
    h.tmdbCredentialBusy = true;
    try {
      await invoke("save_tmdb_read_token", { token: h.tmdbTokenDraft.trim() });
      h.tmdbTokenVisible = true;
      await refreshTmdbCredentialStatus();
      h.setNotice("TMDB token saved in the operating-system keyring.", "success");
    } catch (error) {
      h.setNotice(`Could not save the TMDB token: ${String(error)}`, "error");
    } finally {
      h.tmdbCredentialBusy = false;
    }
  }

  async function revealTmdbCredential() {
    if (!isDesktop() || !h.tmdbCredentialStatus.configured || h.tmdbCredentialBusy) return;
    h.tmdbCredentialBusy = true;
    try {
      h.tmdbTokenDraft = await invoke<string>("reveal_tmdb_read_token");
      h.tmdbTokenVisible = true;
    } catch (error) {
      h.setNotice(`Could not read the saved TMDB token: ${String(error)}`, "error");
    } finally {
      h.tmdbCredentialBusy = false;
    }
  }

  function toggleTmdbCredentialVisibility() {
    if (!h.tmdbTokenDraft) {
      void revealTmdbCredential();
      return;
    }
    h.tmdbTokenVisible = !h.tmdbTokenVisible;
  }

  async function removeTmdbCredential() {
    if (!isDesktop() || h.tmdbCredentialBusy) return;
    h.tmdbCredentialBusy = true;
    try {
      await invoke("remove_tmdb_read_token");
      h.tmdbTokenDraft = "";
      h.tmdbTokenVisible = true;
      await refreshTmdbCredentialStatus();
      h.setNotice("Saved TMDB token removed from the operating-system keyring.", "success");
    } catch (error) {
      h.setNotice(`Could not remove the TMDB token: ${String(error)}`, "error");
    } finally {
      h.tmdbCredentialBusy = false;
    }
  }

  async function saveTvdbCredential() {
    if (!isDesktop() || h.tvdbCredentialBusy) return;
    if (!h.tvdbApiKeyDraft.trim()) {
      h.setNotice("Enter a TheTVDB project API key before saving.", "error");
      return;
    }
    if (!h.tvdbAccessConfirmed) {
      h.setNotice("Confirm that this project has valid TheTVDB API access before saving.", "error");
      return;
    }
    h.tvdbCredentialBusy = true;
    try {
      await invoke("save_tvdb_credentials", {
        apiKey: h.tvdbApiKeyDraft.trim(),
        subscriberPin: h.tvdbSubscriberPinDraft.trim() || null,
        accessConfirmed: true,
      });
      h.tvdbCredentialsVisible = true;
      h.tvdbAccessConfirmed = false;
      await refreshTvdbCredentialStatus();
      h.setNotice("TheTVDB credentials were saved in the operating-system keyring. Their validity will be checked on the next search.", "success");
    } catch (error) {
      h.setNotice(`Could not save TheTVDB credentials: ${String(error)}`, "error");
    } finally {
      h.tvdbCredentialBusy = false;
    }
  }

  async function revealTvdbCredentials() {
    if (!isDesktop() || !h.tvdbCredentialStatus.configured || h.tvdbCredentialBusy) return;
    h.tvdbCredentialBusy = true;
    try {
      const credentials = await invoke<TvdbCredentials>("reveal_tvdb_credentials");
      h.tvdbApiKeyDraft = credentials.apiKey;
      h.tvdbSubscriberPinDraft = credentials.subscriberPin ?? "";
      h.tvdbCredentialsVisible = true;
    } catch (error) {
      h.setNotice(`Could not read the saved TheTVDB credentials: ${String(error)}`, "error");
    } finally {
      h.tvdbCredentialBusy = false;
    }
  }

  function toggleTvdbCredentialVisibility() {
    if (!h.tvdbApiKeyDraft && !h.tvdbSubscriberPinDraft && h.tvdbCredentialStatus.configured) {
      void revealTvdbCredentials();
      return;
    }
    h.tvdbCredentialsVisible = !h.tvdbCredentialsVisible;
  }

  async function removeTvdbCredential() {
    if (!isDesktop() || h.tvdbCredentialBusy) return;
    h.tvdbCredentialBusy = true;
    try {
      await invoke("remove_tvdb_credentials");
      h.tvdbApiKeyDraft = "";
      h.tvdbSubscriberPinDraft = "";
      h.tvdbCredentialsVisible = true;
      h.tvdbAccessConfirmed = false;
      await refreshTvdbCredentialStatus();
      h.setNotice("TheTVDB credentials were removed from the operating-system keyring.", "success");
    } catch (error) {
      h.setNotice(`Could not remove TheTVDB credentials: ${String(error)}`, "error");
    } finally {
      h.tvdbCredentialBusy = false;
    }
  }

  return { refreshTmdbCredentialStatus, refreshTvdbCredentialStatus, saveTmdbCredential, revealTmdbCredential, toggleTmdbCredentialVisibility, removeTmdbCredential, saveTvdbCredential, revealTvdbCredentials, toggleTvdbCredentialVisibility, removeTvdbCredential };
}
