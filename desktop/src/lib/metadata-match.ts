export type MetadataMatchClue = {
  title: string;
  year: string | null;
  yearEnd: string | null;
  kind: "movie" | "tv";
};

export type MetadataMatchCandidate = {
  id: string;
  provider: string;
  kind: "movie" | "tv";
  title: string;
  year: string | null;
};

const PROVIDER_ORDER: Record<string, number> = {
  tmdb: 0,
  tvdb: 1,
  tvmaze: 2,
  wikidata: 3,
  local: 4,
};

export function normalizeMatchTitle(value: string): string {
  return value
    .normalize("NFKC")
    .toLowerCase()
    .replace(/&/g, " and ")
    .match(/[\p{L}\p{N}]+/gu)
    ?.join(" ") ?? "";
}

function years(value: string | null): Set<string> {
  return new Set(value?.match(/(?:19|20)\d{2}/g) ?? []);
}

function clueYears(clue: MetadataMatchClue): Set<string> {
  return new Set([clue.year, clue.yearEnd].filter((value): value is string => Boolean(value)));
}

function titleSimilarity(left: string, right: string): number {
  const leftTokens = new Set(normalizeMatchTitle(left).split(" ").filter(Boolean));
  const rightTokens = new Set(normalizeMatchTitle(right).split(" ").filter(Boolean));
  if (!leftTokens.size || !rightTokens.size) return 0;
  const overlap = [...leftTokens].filter((token) => rightTokens.has(token)).length;
  return overlap / new Set([...leftTokens, ...rightTokens]).size;
}

function yearRank(candidate: MetadataMatchCandidate, clue: MetadataMatchClue): number {
  const actual = years(candidate.year);
  const expected = clueYears(clue);
  if (!actual.size || !expected.size) return 1;
  return [...actual].some((year) => expected.has(year)) ? 0 : 2;
}

function titleRank(candidate: MetadataMatchCandidate, clue: MetadataMatchClue): number {
  const expected = normalizeMatchTitle(clue.title);
  const actual = normalizeMatchTitle(candidate.title);
  if (expected && expected === actual) return 0;
  const similarity = titleSimilarity(clue.title, candidate.title);
  if (similarity >= 0.8) return 1;
  if (similarity >= 0.5) return 2;
  return 3;
}

function agreementCounts<T extends MetadataMatchCandidate>(results: T[]): Map<string, number> {
  const providersByRecord = new Map<string, Set<string>>();
  for (const result of results) {
    const normalized = normalizeMatchTitle(result.title);
    const releaseYear = [...years(result.year)].sort()[0] ?? "";
    if (!normalized) continue;
    const key = `${result.kind}\0${normalized}\0${releaseYear}`;
    const providers = providersByRecord.get(key) ?? new Set<string>();
    providers.add(result.provider);
    providersByRecord.set(key, providers);
  }
  return new Map(
    [...providersByRecord.entries()].map(([key, providers]) => [key, providers.size]),
  );
}

function agreementKey(candidate: MetadataMatchCandidate): string {
  const normalized = normalizeMatchTitle(candidate.title);
  const releaseYear = [...years(candidate.year)].sort()[0] ?? "";
  return `${candidate.kind}\0${normalized}\0${releaseYear}`;
}

export function rankMetadataMatches<T extends MetadataMatchCandidate>(
  results: T[],
  clue: MetadataMatchClue,
): T[] {
  const agreements = agreementCounts(results);
  return [...results].sort((left, right) =>
    Number(left.kind !== clue.kind) - Number(right.kind !== clue.kind)
    || yearRank(left, clue) - yearRank(right, clue)
    || titleRank(left, clue) - titleRank(right, clue)
    || (agreements.get(agreementKey(right)) ?? 0) - (agreements.get(agreementKey(left)) ?? 0)
    || titleSimilarity(clue.title, right.title) - titleSimilarity(clue.title, left.title)
    || (PROVIDER_ORDER[left.provider] ?? 100) - (PROVIDER_ORDER[right.provider] ?? 100)
    || left.title.localeCompare(right.title)
    || left.id.localeCompare(right.id),
  );
}

/**
 * Auto-associate only exact normalized titles with an exact filename year and
 * no same-title conflicting year or duplicate same-provider record. This is a
 * deterministic safety rule, not a probability or a claimed precision rate.
 */
export function automaticMetadataMatch<T extends MetadataMatchCandidate>(
  results: T[],
  clue: MetadataMatchClue,
): T | null {
  const expectedTitle = normalizeMatchTitle(clue.title);
  const expectedYears = clueYears(clue);
  if (!expectedTitle || !expectedYears.size) return null;

  const sameTitle = results.filter((candidate) =>
    candidate.kind === clue.kind && normalizeMatchTitle(candidate.title) === expectedTitle,
  );
  if (sameTitle.some((candidate) => {
    const actualYears = years(candidate.year);
    return actualYears.size > 0 && ![...actualYears].some((year) => expectedYears.has(year));
  })) return null;

  const matching = sameTitle.filter((candidate) => {
    const actualYears = years(candidate.year);
    return [...actualYears].some((year) => expectedYears.has(year));
  });
  if (!matching.length) return null;

  const providersWithResults = [...new Set(results
    .filter((candidate) => candidate.kind === clue.kind)
    .map((candidate) => candidate.provider))];
  if (providersWithResults.length > 1) {
    for (const provider of providersWithResults) {
      const providerTop = rankMetadataMatches(
        results.filter((candidate) => candidate.kind === clue.kind && candidate.provider === provider),
        clue,
      )[0];
      if (!providerTop
          || normalizeMatchTitle(providerTop.title) !== expectedTitle
          || ![...years(providerTop.year)].some((year) => expectedYears.has(year))) {
        return null;
      }
    }
  }

  const countByProvider = new Map<string, number>();
  for (const candidate of matching) {
    countByProvider.set(candidate.provider, (countByProvider.get(candidate.provider) ?? 0) + 1);
  }
  if ([...countByProvider.values()].some((count) => count > 1)) return null;

  return rankMetadataMatches(matching, clue)[0] ?? null;
}

export function metadataMatchLabel(
  candidate: MetadataMatchCandidate,
  clue: MetadataMatchClue,
): { text: string; conflict: boolean } {
  const expectedTitle = normalizeMatchTitle(clue.title);
  const actualTitle = normalizeMatchTitle(candidate.title);
  const expectedYears = clueYears(clue);
  const actualYears = years(candidate.year);
  const yearConflict = expectedYears.size > 0 && actualYears.size > 0
    && ![...actualYears].some((year) => expectedYears.has(year));
  if (yearConflict) {
    return {
      text: `Year differs · filename ${[...expectedYears].join("–")}, source ${[...actualYears].join("–")}`,
      conflict: true,
    };
  }
  if (expectedTitle && expectedTitle === actualTitle) {
    return {
      text: `Exact title${expectedYears.size && actualYears.size ? " · year matches filename" : " · review the year"}`,
      conflict: false,
    };
  }
  return {
    text: `Similar title${[...actualYears].some((year) => expectedYears.has(year)) ? " · year matches filename" : " · review match"}`,
    conflict: false,
  };
}
