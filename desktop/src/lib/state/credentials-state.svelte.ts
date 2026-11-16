// ./desktop/src/lib/state/credentials-state.svelte.ts
// Metadata-provider credential drafts, visibility and busy flags (TMDB token, TheTVDB key and PIN).
// A plain rune class: its instance is the typed host the controller works on, so the route needs no
// per-field accessors. Members marked `declare` are supplied by the route (see withGetters).
import type { TmdbCredentialStatus, TvdbCredentialStatus } from "../ui-contracts.js";

export class CredentialsState {
  tmdbCredentialStatus = $state<TmdbCredentialStatus>({ configured: false, secureStoreAvailable: false, message: null });
  tmdbTokenDraft = $state("");
  tmdbTokenVisible = $state(true);
  tmdbCredentialBusy = $state(false);
  tvdbCredentialStatus = $state<TvdbCredentialStatus>({ configured: false, subscriberPinConfigured: false, secureStoreAvailable: false, message: null });
  tvdbApiKeyDraft = $state("");
  tvdbSubscriberPinDraft = $state("");
  tvdbCredentialsVisible = $state(true);
  tvdbAccessConfirmed = $state(false);
  tvdbCredentialBusy = $state(false);
  setNotice: (message: string, kind?: "info" | "success" | "error") => void = () => {};
}
