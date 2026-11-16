// Library page cards, crumbs and organizer templates.

export type PosterMatchState = "matching" | "matched" | "suggested" | "local" | "none" | "error";

export interface LibraryCrumb {
  label: string;
  onSelect: (() => void) | null;
}

export interface LibraryPosterCard {
  path: string;
  filename: string;
  relativePath: string;
  selected: boolean;
  title: string;
  year: string;
  proposedName: string | null;
  proposedNeedsReview: boolean;
  posterUrl: string | null;
  localArtworkPath: string;
  matchState: PosterMatchState;
  matchLabel: string;
  provider: string;
  animated: boolean;
  statusLabel: string;
  sourceBytes: string;
  profileLine: string;
  streamLine: string;
  memoryLine: string;
  outputLine: string;
  outputIsCustom: boolean;
  error: string;
  warning: string;
  stale: string;
  runLabel: string;
  progressLabel: string;
  progressPercent: number | null;
  kind: "media" | "series" | "season";
  seasonNumber: number | null;
  mediaPaths: string[];
  selection: "none" | "some" | "all";
  animation: "none" | "some" | "all";
  summary: string;
  reviewLabel: string;
}

export interface OrganizerTemplates {
    movie: string;
    series: string;
    season: string;
    episode: string;
}
