// ./desktop/src/lib/review-notes.ts
// Pure library-review helpers extracted from the route (review slice A): inherited certificates and
// comments with the notes shown for them, review field patching, poster choices, review lookup by
// folder, review headings and paths. Everything the route used to read from its state is a parameter.
// No IPC, no DOM. Tested by pnpm test:review-notes.

import { blankSeasonReview } from './review-factories.ts';
import { queuePathKey } from './queue-paths.ts';
import type { LibrarySeriesGroup } from './library-browse.ts';
import type { QueueSeasonReview, QueueSeriesReview, QueueTvEpisode } from './types.js';

export type LibraryTvReviewTarget = { level: "series" | "season" | "episode"; seriesRoot: string; season: number | null; episodePath: string | null };
export type LocalLibraryRecord = { artwork: { name: string; path: string; role: string }[] };

export function seasonPremiereYear(review: QueueSeriesReview | undefined, season: number | null): string {
  if (!review || season === null) return "";
  const dates = [...review.episodes, ...Object.values(review.episodeAssignments)]
    .filter((episode) => episode.season === season)
    .map((episode) => episode.airdate?.trim() ?? "")
    .filter((airdate) => /^\d{4}-\d{2}-\d{2}$/.test(airdate))
    .sort();
  return dates[0]?.slice(0, 4) ?? "";
}

export function sharedCertificateFor(target: LibraryTvReviewTarget, review: QueueSeriesReview): string {
  const seriesCertificate = review.certification.trim();
  if (target.level !== "episode") return seriesCertificate;
  const seasonCertificate = target.season === null ? "" : review.seasonRecords?.[String(target.season)]?.certification.trim() ?? "";
  return seasonCertificate || seriesCertificate;
}

export function ownCertificateFor(target: LibraryTvReviewTarget, review: QueueSeriesReview): string {
  if (target.level === "episode" && target.episodePath) return review.episodeAssignments[target.episodePath]?.certification?.trim() ?? "";
  if (target.level === "season" && target.season !== null) return review.seasonRecords?.[String(target.season)]?.certification.trim() ?? "";
  return "";
}

export function certificateNoteFor(target: LibraryTvReviewTarget, review: QueueSeriesReview): string {
  if (target.level === "series") return "";
  if (ownCertificateFor(target, review)) return target.level === "episode" ? "Saved for this episode." : "Saved for this season.";
  if (!sharedCertificateFor(target, review)) return "No certificate is published.";
  if (target.level === "episode" && target.season !== null && review.seasonRecords?.[String(target.season)]?.certification.trim()) return "Using this season's certificate.";
  return "Using the series certificate.";
}

export function seasonPlotNote(review: QueueSeriesReview, season: QueueSeasonReview | undefined): string {
  if (season?.plot.trim()) return "Saved for this season.";
  if (review.plot.trim()) return "This season has no plot of its own. Showing the series description.";
  return "No season description is published.";
}

export function seasonRatingNote(review: QueueSeriesReview, season: QueueSeasonReview | undefined): string {
  if (season?.ratingText.trim()) return "Saved for this season.";
  if (review.ratingText.trim()) return "This season has no score of its own. Showing the series score.";
  return "No season score is published.";
}

/** `settingsComment` is the app-wide default comment from Settings. */
export function sharedCommentFor(target: LibraryTvReviewTarget, review: QueueSeriesReview, settingsComment: string | undefined): string {
  const globalComment = settingsComment?.trim() ?? "";
  const seriesComment = review.comment?.trim() ?? "";
  if (target.level === "series") return globalComment;
  if (target.level === "season") return seriesComment || globalComment;
  const seasonComment = target.season === null ? "" : review.seasonRecords?.[String(target.season)]?.comment?.trim() ?? "";
  return seasonComment || seriesComment || globalComment;
}

export function ownCommentFor(target: LibraryTvReviewTarget, review: QueueSeriesReview): string {
  if (target.level === "episode" && target.episodePath) return review.episodeAssignments[target.episodePath]?.comment?.trim() ?? "";
  if (target.level === "season" && target.season !== null) return review.seasonRecords?.[String(target.season)]?.comment?.trim() ?? "";
  return review.comment?.trim() ?? "";
}

export function commentNoteFor(target: LibraryTvReviewTarget, review: QueueSeriesReview): string {
  if (ownCommentFor(target, review)) {
    if (target.level === "episode") return "Saved for this episode.";
    if (target.level === "season") return "Saved for this season. Episodes without their own comment use it.";
    return "Saved for this series. Seasons and episodes without their own comment use it.";
  }
  if (target.level === "episode") {
    const seasonComment = target.season === null ? "" : review.seasonRecords?.[String(target.season)]?.comment?.trim() ?? "";
    if (seasonComment) return "Using this season's comment. Edit it to save one for this episode.";
    if (review.comment?.trim()) return "Using the series comment. Edit it to save one for this episode.";
  }
  if (target.level === "season" && review.comment?.trim()) return "Using the series comment. Edit it to save one for this season.";
  return "Using the comment from Settings. Edit it to save one here.";
}

export function resolvedFileComment(series: QueueSeriesReview, assignment: QueueTvEpisode | undefined): string {
  const episodeComment = assignment?.comment?.trim() ?? "";
  if (episodeComment) return episodeComment;
  const seasonComment = assignment ? series.seasonRecords?.[String(assignment.season)]?.comment?.trim() ?? "" : "";
  if (seasonComment) return seasonComment;
  return series.comment?.trim() ?? "";
}

/** `seriesReviewsByRoot` is keyed by `queuePathKey` of each reviewed series folder. */
export function reviewedSeriesForPath<T>(path: string, seriesReviewsByRoot: ReadonlyMap<string, T>): T | undefined {
  let current = queuePathKey(path);
  while (current) {
    const review = seriesReviewsByRoot.get(current);
    if (review) return review;
    const separator = current.lastIndexOf("/");
    if (separator < 0) break;
    if (separator === 0) current = "/";
    else if (separator === 2 && /^[a-z]:\//i.test(current)) current = current.slice(0, 3);
    else current = current.slice(0, separator);
    if (current === "/" || /^[a-z]:\/$/i.test(current)) {
      const rootReview = seriesReviewsByRoot.get(current);
      if (rootReview) return rootReview;
      break;
    }
  }
  return undefined;
}

/** Returns the review with one field of the reviewed series, season or episode changed; the same object when the edit is rejected. */
export function applyTvField(
  review: QueueSeriesReview,
  target: LibraryTvReviewTarget,
  field: string,
  value: string,
  episodeDraft: (path: string) => QueueTvEpisode,
): QueueSeriesReview {
  if (target.level === "episode" && target.episodePath) {
    const current = review.episodeAssignments[target.episodePath] ?? episodeDraft(target.episodePath);
    const next = { ...current };
    if (field === "title") next.title = value;
    if (field === "plot") next.summary = value;
    if (field === "airdate") next.airdate = value || null;
    if (field === "comment") next.comment = value;
    if (field === "certification") {
      const inherited = sharedCertificateFor(target, review);
      next.certification = inherited && value.trim() === inherited ? "" : value;
    }
    if (field === "season") {
      const number = Number(value);
      if (!Number.isInteger(number) || number < 0) return review;
      next.season = number;
    }
    if (field === "episode") {
      const number = Number(value);
      if (!Number.isInteger(number) || number < 1) return review;
      next.episode = number;
    }
    return { ...review, episodeAssignments: { ...review.episodeAssignments, [target.episodePath]: next } };
  }
  if (target.level === "season" && target.season !== null) {
    const seasonKey = String(target.season);
    const current = review.seasonRecords[seasonKey] ?? blankSeasonReview(`Season ${target.season}`, review);
    const inherited = field === "plot" ? review.plot.trim() : field === "ratingText" ? review.ratingText.trim() : field === "certification" ? review.certification.trim() : "";
    const nextValue = inherited && value.trim() === inherited ? "" : value;
    return { ...review, seasonRecords: { ...review.seasonRecords, [seasonKey]: { ...current, [field]: nextValue } } };
  }
  return { ...review, [field]: value };
}

export function posterPathsForReview(group: LibrarySeriesGroup, season: number | null, localRecords: Record<string, LocalLibraryRecord>): string[] {
  const seriesArtwork = localRecords[queuePathKey(group.root)]?.artwork ?? [];
  const seasonDirectory = season === null ? "" : group.seasons.find((item) => item.season === season)?.directory ?? "";
  const seasonArtwork = seasonDirectory ? localRecords[queuePathKey(seasonDirectory)]?.artwork ?? [] : [];
  const images = season === null ? seriesArtwork : [...seasonArtwork, ...seriesArtwork];
  const filtered = images.filter((image) => !/fanart|banner|landscape|clearlogo|logo/i.test(`${image.role} ${image.name}`));
  const chosen = (filtered.length ? filtered : images).map((image) => image.path);
  return [...new Set(chosen)];
}

export function reviewPosterChoices(
  group: LibrarySeriesGroup,
  season: number | null,
  review: QueueSeriesReview | undefined,
  localRecords: Record<string, LocalLibraryRecord>,
  displayPosterUrl: (path: string) => string | null,
): { path: string; url: string }[] {
  const local = posterPathsForReview(group, season, localRecords).map((path) => ({ path, url: displayPosterUrl(path) ?? "" }));
  const remote = (review?.artworkCandidates ?? [])
    .map((image) => image.imageUrl || image.thumbnailUrl)
    .filter((url) => url.startsWith("https://"))
    .map((url) => ({ path: url, url }));
  const seen = new Set(local.map((item) => item.path));
  const choices = [...local, ...remote.filter((item) => !seen.has(item.path))];
  const selected = season === null ? review?.localPosterPath ?? "" : review?.localSeasonPosterPaths?.[String(season ?? "")] ?? "";
  if (selected.startsWith("https://") && !choices.some((item) => item.path === selected)) {
    choices.unshift({ path: selected, url: selected });
  }
  return choices;
}

export function tvReviewPaths(target: LibraryTvReviewTarget, group: LibrarySeriesGroup | undefined): string[] {
  if (target.level === "episode" && target.episodePath) return [target.episodePath];
  if (target.level === "season" && target.season !== null) return group?.seasons.find((item) => item.season === target.season)?.episodePaths ?? [];
  return group?.episodePaths ?? [];
}

/** `review` is the saved series review for the target's folder, when there is one. */
export function tvReviewHeading(target: LibraryTvReviewTarget, review: QueueSeriesReview | undefined, episodeDraft: (path: string) => QueueTvEpisode): string {
  if (!review) return target.episodePath ? episodeDraft(target.episodePath).title : "";
  if (target.level === "series") return review.title;
  if (target.level === "season" && target.season !== null) return review.seasonRecords?.[String(target.season)]?.title || `Season ${target.season}`;
  if (target.episodePath) return review.episodeAssignments?.[target.episodePath]?.title?.trim() || episodeDraft(target.episodePath).title;
  return "";
}
