// ./desktop/src/lib/run-overrides.ts
// Per-file poster and metadata overrides sent to the engine with start_run, extracted from the
// route. The review state the route owns is injected through getters, so the rules (local poster
// precedence, season resolution, comment precedence, bounded text) are unit-tested without the
// route. No IPC, no DOM. Tested by pnpm test:run-overrides.

import { episodeOutputText } from './library-browse.ts';
import { queuePathKey } from './queue-paths.ts';
import type { MediaEntry, QueueMovieReview, QueueSeriesReview, QueueTvEpisode } from './types.js';

export interface RunOverridesDeps {
  getMovieReviews: () => Record<string, QueueMovieReview>;
  reviewedSeriesForPath: (path: string) => QueueSeriesReview | undefined;
  getLocalPosters: () => Record<string, string>;
  getSettingsComment: () => string | undefined;
  resolvedFileComment: (series: QueueSeriesReview, assignment: QueueTvEpisode | undefined) => string;
}

export function createRunOverrides(deps: RunOverridesDeps) {
  const { getMovieReviews, reviewedSeriesForPath, getLocalPosters, getSettingsComment, resolvedFileComment } = deps;

  function selectedPosterOverrides(entries: MediaEntry[]): Record<string, string> {
    const overrides: Record<string, string> = {};
    for (const entry of entries) {
      const movieReview = getMovieReviews()[entry.path];
      if (movieReview?.localPosterPath && !movieReview.localPosterPath.startsWith("https://")) {
        overrides[entry.path] = movieReview.localPosterPath;
        continue;
      }
      const seriesReview = reviewedSeriesForPath(entry.path);
      if (!seriesReview) continue;
      const episodeAssignment = seriesReview.episodeAssignments[entry.path] ??
        Object.entries(seriesReview.episodeAssignments).find(([path]) => queuePathKey(path) === queuePathKey(entry.path))?.[1];
      const parentName = entry.path.split(/[\\/]/).at(-2) ?? "";
      const folderSeason = parentName.match(/^season[\s._-]*(\d+)$/i)?.[1];
      const season = episodeAssignment?.season ?? (folderSeason ? Number(folderSeason) : null);
      const seasonPoster = season === null ? "" : seriesReview.localSeasonPosterPaths[String(season)] ?? "";
      const seasonDirectory = entry.path.replace(/[/\\][^/\\]+$/, "");
      const seasonOnDisk = getLocalPosters()[queuePathKey(seasonDirectory)] ?? "";
      const seasonFile = [seasonPoster, seasonOnDisk].find((path) => path && !path.startsWith("https://")) ?? "";
      if (seasonFile) {
        overrides[entry.path] = seasonFile;
        continue;
      }
      // A provider URL means this season already has its own art. Do not
      // substitute the series poster; the engine uses Season NN/poster.* on disk.
      if (seasonPoster.startsWith("https://")) continue;
      const seriesPoster = seriesReview.localPosterPath ?? "";
      if (seriesPoster && !seriesPoster.startsWith("https://")) overrides[entry.path] = seriesPoster;
    }
    return overrides;
  }

  function selectedMetadataOverrides(entries: MediaEntry[]): Record<string, Record<string, string | string[]>> {
    const overrides: Record<string, Record<string, string | string[]>> = {};
    const addText = (target: Record<string, string | string[]>, key: string, value: string | null | undefined) => {
      const normalized = value?.trim();
      if (normalized) target[key] = normalized;
    };
    const addGenres = (target: Record<string, string | string[]>, value: string) => {
      const genres = value.split(/[;,]/).map((genre) => genre.trim()).filter(Boolean).slice(0, 50);
      if (genres.length) target.genres = genres;
    };

    for (const entry of entries) {
      const movie = getMovieReviews()[entry.path];
      if (movie) {
        const fields: Record<string, string | string[]> = {};
        addText(fields, "title", movie.title);
        addText(fields, "year", movie.year);
        addText(fields, "certification", movie.certification);
        addText(fields, "short_description", movie.shortDescription);
        addText(fields, "plot", movie.plot);
        addGenres(fields, movie.genresText);
        addText(fields, "rating", movie.ratingText);
        addText(fields, "source_url", movie.sourceUrl);
        const movieComment = movie.comment?.trim() ?? "";
        if (movieComment && movieComment !== getSettingsComment()?.trim()) addText(fields, "comment", movieComment);
        if (Object.keys(fields).length) overrides[entry.path] = fields;
        continue;
      }

      const series = reviewedSeriesForPath(entry.path);
      if (!series) continue;
      const assignment = series.episodeAssignments[entry.path] ??
        Object.entries(series.episodeAssignments).find(([path]) => queuePathKey(path) === queuePathKey(entry.path))?.[1];
      const fields: Record<string, string | string[]> = {};
      addText(fields, "title", assignment?.title ?? entry.name.replace(/\.[^.]+$/, ""));
      addText(fields, "show_title", series.title);
      addText(fields, "year", series.year);
      const seasonCertificate = assignment ? series.seasonRecords?.[String(assignment.season)]?.certification.trim() ?? "" : "";
      addText(fields, "certification", assignment?.certification?.trim() || seasonCertificate || series.certification);
      const episodeText = episodeOutputText(assignment?.summary, assignment?.sourceUrl);
      addText(fields, "short_description", episodeText.summary);
      addText(fields, "plot", episodeText.synopsis);
      addGenres(fields, series.genresText);
      addText(fields, "source_url", episodeText.sourceUrl);
      if (assignment) {
        fields.season = String(assignment.season);
        fields.episode = String(assignment.episode);
        addText(fields, "airdate", assignment.airdate);
      }
      const individualComment = resolvedFileComment(series, assignment);
      if (individualComment && individualComment !== getSettingsComment()?.trim()) addText(fields, "comment", individualComment);
      if (Object.keys(fields).length) overrides[entry.path] = fields;
    }
    return overrides;
  }

  return { selectedPosterOverrides, selectedMetadataOverrides };
}
