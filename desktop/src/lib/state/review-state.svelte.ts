// ./desktop/src/lib/state/review-state.svelte.ts
// Title/poster reviews, match suggestions, lookup status and the metadata review dialog.
// A plain rune class: its instance is the typed host the controller works on, so the route needs no
// per-field accessors. Members marked `declare` are supplied by the route (see withGetters).
import type { QueueMovieReview, QueueSeriesReview, QueueMetadataSuggestion, MetadataLookupStatus } from "../types.js";
import type { LibraryTvReviewTarget } from "../review-notes.ts";

export class ReviewState {
  queueMovieReviews = $state<Record<string, QueueMovieReview>>({});
  queueSeriesReviews = $state<Record<string, QueueSeriesReview>>({});
  queueMatchSuggestions = $state<Record<string, QueueMetadataSuggestion>>({});
  libraryTvReview = $state<LibraryTvReviewTarget | null>(null);
  metadataFocusPath = $state<string | null>(null);
  metadataDialogOpen = $state(false);
  metadataDialogElement = $state<HTMLDialogElement | null>(null);
  metadataLookupPending = $state(false);
  metadataLookupStatus = $state<MetadataLookupStatus | null>(null);
}
