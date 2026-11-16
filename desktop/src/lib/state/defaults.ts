// ./desktop/src/lib/state/defaults.ts
// Default values shared by the route and the state classes.

import type { OrganizerTemplates } from "../types.js";

export const defaultOrganizerTemplates: OrganizerTemplates = {
  movie: "{title} [[({year} {certification})]]{ext}",
  series: "{series_title}",
  season: "Season {season:02}",
  episode: "S{season:02}E{episode:02}.{episode_title}{ext}",
};
export const defaultFilenameExcludePatterns = [
  "old", ".*", ".renamethethings", "@eadir", "extras", "extra", "trailers", "trailer",
  "samples", "sample", "behind the scenes", "behindthescenes", "deleted scenes",
  "deleted", "interviews", "interview", "scenes", "clips", "clip", "featurettes",
  "featurette", "shorts", "short", "other", "theme-music", "thememusic", "backdrops",
  "extrafanart",
];
