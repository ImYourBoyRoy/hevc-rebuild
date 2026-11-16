// Queue metadata reviews: artwork candidates, suggestions and movie, series, season and episode reviews.

export interface QueueArtworkCandidate {
  id: string;
  provider: string;
  title: string;
  imageUrl: string;
  thumbnailUrl: string;
  sourceUrl: string;
  creator: string | null;
  license: string | null;
  licenseUrl: string | null;
  attribution: string | null;
}

export interface QueueMetadataSuggestion {
  kind: 'movie' | 'tv';
  status: 'searching' | 'ready' | 'error';
  title: string | null;
  year: string | null;
  provider: string | null;
  recordId: string | null;
  sourceUrl: string | null;
  description: string | null;
  autoAssociated: boolean;
  artwork: QueueArtworkCandidate | null;
  error: string;
}

export interface MetadataLookupStatus {
  status: 'searching' | 'ready' | 'error';
  done: number;
  total: number;
  label: string;
  message: string;
  sourceRoot?: string;
}

export interface QueueMovieReview {
  title: string;
  year: string;
  certification: string;
  recordId: string;
  shortDescription: string;
  plot: string;
  genresText: string;
  ratingText: string;
  sourceUrl: string;
  comment?: string;
  artworkCandidates: QueueArtworkCandidate[];
  posterId: string;
  localPosterPath: string;
}

export interface QueueTvEpisode {
  season: number;
  episode: number;
  title: string;
  airdate: string | null;
  summary: string | null;
  sourceUrl: string;
  comment?: string;
  certification?: string;
}

export interface QueueSeasonReview {
  title: string;
  year: string;
  certification: string;
  plot: string;
  genresText: string;
  ratingText: string;
  comment?: string;
}

export interface QueueSeriesReview {
  title: string;
  year: string;
  certification: string;
  recordId: string;
  shortDescription: string;
  plot: string;
  genresText: string;
  ratingText: string;
  sourceUrl: string;
  comment?: string;
  artworkCandidates: QueueArtworkCandidate[];
  posterId: string;
  localPosterPath: string;
  localSeasonPosterPaths: Record<string, string>;
  seasonRecords: Record<string, QueueSeasonReview>;
  episodes: QueueTvEpisode[];
  episodeAssignments: Record<string, QueueTvEpisode>;
}
