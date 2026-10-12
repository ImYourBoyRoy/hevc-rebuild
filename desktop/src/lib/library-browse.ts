// ./desktop/src/lib/library-browse.ts
// Groups a scanned TV folder into series, seasons, and episodes for the library.
// Movies stay out of these groups. Episode art is chosen later from the season poster.

export interface LibrarySeasonGroup {
  season: number;
  directory: string;
  label: string;
  episodePaths: string[];
}

export interface LibrarySeriesGroup {
  root: string;
  folderName: string;
  seasons: LibrarySeasonGroup[];
  episodePaths: string[];
}

export interface EpisodeIdentity {
  season: number;
  episode: number;
  title: string;
}

interface GroupableEntry {
  path: string;
  relative_path: string;
  name: string;
}

export function episodeIdentity(filename: string): EpisodeIdentity | null {
  const stem = filename.replace(/\.[^.]+$/u, "");
  const match = /^s(\d{1,4})e(\d{1,4})(?:[.\s_-]+(.+))?$/iu.exec(stem);
  if (!match) return null;
  const title = (match[3] ?? "").replace(/[._]+/gu, " ").replace(/\s+/gu, " ").trim();
  return {
    season: Number(match[1]),
    episode: Number(match[2]),
    title,
  };
}

export interface CatalogEpisodeText {
  season: number;
  episode: number;
  title: string;
  airdate: string | null;
  summary: string | null;
  sourceUrl: string;
}

export interface EpisodeTextFields {
  season: number;
  episode: number;
  title: string;
  airdate: string | null;
  summary: string | null;
  sourceUrl: string;
}

export interface SeasonTextFields {
  title: string;
  year: string;
  certification: string;
  plot: string;
  genresText: string;
  ratingText: string;
  comment?: string;
}

export interface SeasonProviderText {
  number: number;
  title: string;
  overview: string;
  rating: number | null;
}

function boundedText(value: string, maximum: number): string {
  const trimmed = value.trim();
  return trimmed.length > maximum ? trimmed.slice(0, maximum) : trimmed;
}

function httpsText(value: string, maximum: number): string {
  const trimmed = value.trim();
  if (!trimmed.startsWith("https://") || trimmed.length > maximum) return "";
  return trimmed;
}

function audienceScoreText(rating: number | null): string {
  if (typeof rating !== "number" || !Number.isFinite(rating) || rating <= 0 || rating > 10) return "";
  const rounded = Math.round(rating * 10) / 10;
  return Number.isInteger(rounded) ? String(rounded) : rounded.toFixed(1);
}

export function fillEpisodeFromCatalog<T extends EpisodeTextFields>(
  current: T,
  match: CatalogEpisodeText | undefined,
  seriesTitle: string,
  seriesPlot: string,
  filenameTitle: string,
): T {
  if (!match) return current;
  const providerTitle = boundedText(match.title, 1024);
  const providerPlot = boundedText(match.summary ?? "", 16_384);
  const providerDate = boundedText(match.airdate ?? "", 64);
  const currentTitle = current.title.trim();
  const currentPlot = current.summary?.trim() ?? "";
  const showTitle = seriesTitle.trim();
  const showPlot = seriesPlot.trim();
  const parsedTitle = filenameTitle.trim();
  const automaticTitle = !currentTitle || currentTitle === showTitle || (parsedTitle !== "" && currentTitle === parsedTitle);
  const automaticPlot = !currentPlot || (showPlot !== "" && currentPlot === showPlot);
  const title = automaticTitle ? (providerTitle || parsedTitle || current.title) : current.title;
  const summary = automaticPlot ? (providerPlot || (currentPlot === showPlot ? "" : current.summary)) : current.summary;
  const sourceUrl = current.sourceUrl.trim() ? current.sourceUrl : httpsText(match.sourceUrl, 4096);
  return {
    ...current,
    season: current.season || match.season,
    episode: current.episode || match.episode,
    title: boundedText(title, 1024) || current.title,
    summary: summary === null || summary === undefined ? summary : boundedText(summary, 16_384),
    airdate: current.airdate?.trim() ? current.airdate : providerDate || current.airdate,
    sourceUrl,
  };
}

export function episodeOutputText(summary: string | null | undefined, sourceUrl: string | null | undefined): { summary: string; synopsis: string; sourceUrl: string } {
  const plot = boundedText(summary ?? "", 16_384);
  return { summary: plot, synopsis: plot, sourceUrl: httpsText(sourceUrl ?? "", 4096) };
}

export function fillSeasonFromProvider<T extends SeasonTextFields>(
  current: T | undefined,
  provider: SeasonProviderText,
  seriesGenres: string,
): T {
  const label = `Season ${String(provider.number).padStart(2, "0")}`;
  const generic = new Set([label.toLowerCase(), `season ${provider.number}`, `series ${provider.number}`]);
  const providerTitle = boundedText(provider.title, 1024);
  const title = current?.title.trim()
    ? current.title
    : providerTitle && !generic.has(providerTitle.toLowerCase())
      ? providerTitle
      : label;
  const plot = current?.plot.trim() ? current.plot : boundedText(provider.overview, 65_536);
  const ratingText = current?.ratingText.trim() ? current.ratingText : audienceScoreText(provider.rating);
  const base = {
    title,
    year: current?.year ?? "",
    certification: current?.certification ?? "",
    plot,
    genresText: current ? current.genresText : seriesGenres,
    ratingText,
    comment: current?.comment,
  };
  return { ...(current ?? {}), ...base } as T;
}

export function seasonFromDirectory(name: string): number | null {
  if (/^specials$/iu.test(name)) return 0;
  const match = /^season[\s._-]*(\d{1,4})$/iu.exec(name);
  return match ? Number(match[1]) : null;
}

function joinPath(base: string, part: string): string {
  const separator = base.includes("\\") && !base.includes("/") ? "\\" : "/";
  return `${base.replace(/[\\/]+$/u, "")}${separator}${part}`;
}

function folderName(path: string): string {
  const parts = path.split(/[\\/]/u).filter(Boolean);
  return parts.at(-1) ?? path;
}

export function groupLibraryEntries(sourceRoot: string, entries: GroupableEntry[]): LibrarySeriesGroup[] {
  const series = new Map<string, LibrarySeriesGroup>();
  for (const entry of entries) {
    const relativeParts = entry.relative_path.split(/[\\/]/u).filter(Boolean);
    relativeParts.pop();
    const seasonIndex = relativeParts.findIndex((part) => seasonFromDirectory(part) !== null);
    const identity = episodeIdentity(entry.name);
    if (seasonIndex < 0 && !identity) continue;

    const seriesParts = seasonIndex >= 0 ? relativeParts.slice(0, seasonIndex) : relativeParts;
    const root = seriesParts.reduce(joinPath, sourceRoot);
    const season = seasonIndex >= 0 ? seasonFromDirectory(relativeParts[seasonIndex]) ?? identity?.season ?? 0 : identity?.season ?? 0;
    const directory = seasonIndex >= 0 ? joinPath(root, relativeParts[seasonIndex]) : "";
    const group = series.get(root) ?? {
      root,
      folderName: seriesParts.length ? seriesParts.at(-1) ?? folderName(root) : folderName(root),
      seasons: [],
      episodePaths: [],
    };
    group.episodePaths.push(entry.path);
    let seasonGroup = group.seasons.find((item) => item.season === season);
    if (!seasonGroup) {
      seasonGroup = {
        season,
        directory,
        label: season === 0 ? "Specials" : `Season ${String(season).padStart(2, "0")}`,
        episodePaths: [],
      };
      group.seasons.push(seasonGroup);
    } else if (!seasonGroup.directory && directory) {
      seasonGroup.directory = directory;
    }
    seasonGroup.episodePaths.push(entry.path);
    series.set(root, group);
  }

  return [...series.values()]
    .map((group) => ({
      ...group,
      seasons: group.seasons.sort((left, right) => left.season - right.season),
    }))
    .sort((left, right) => left.folderName.localeCompare(right.folderName));
}

export function seasonPosterFromArtwork(images: { name: string; path: string }[], season: number): string {
  const padded = String(season).padStart(2, "0");
  return images.find((image) => new RegExp(`season\\s*${padded}-poster`, "iu").test(image.name))?.path ?? "";
}

export function preferredLocalPoster(images: { name: string; path: string }[], season: number | null): string {
  if (season !== null) {
    const seasonal = seasonPosterFromArtwork(images, season);
    if (seasonal) return seasonal;
  }
  return images.find((image) => /^poster\./iu.test(image.name))?.path
    ?? images.find((image) => /poster|cover|folder/iu.test(image.name))?.path
    ?? "";
}
