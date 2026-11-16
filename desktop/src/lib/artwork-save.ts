// ./desktop/src/lib/artwork-save.ts
// Poster saving extracted from MetadataWorkbench (T10.7 slice 9): staging a provider poster for saving, the planned file paths, saving it
// beside media or into the private cache and attaching it to the reviewed selections, choosing and saving a local poster for a film, the
// film poster shortcuts and the selected artwork lookups. The state and props are reached through a typed host, the bridge and dialog are
// injected so every branch is unit-tested in node (pnpm test:artwork-save). The code is the component's code moved verbatim.

import type { Artwork, ArtworkQueueBinding, ArtworkSaveResult, ArtworkTarget, Detail, LocalArtworkFile, MovieAssignment, OrganizerFile, OrganizerScan, PendingArtworkSave, PendingLocalPoster, Provider, Result, SeriesAssignment } from "./metadata-workbench-types.js";
import { artworkFileExtension, movieArtworkTarget } from "./metadata-candidates.ts";
import { basename, joinPath } from "./organizer-analysis.ts";

export interface ArtworkSaveHost {
  readonly savePosterBesideMedia: boolean;
  movieAssignments: Record<string, MovieAssignment>;
  seriesAssignments: Record<string, SeriesAssignment>;
  selectedArtwork: Record<string, string>;
  organizerScan: OrganizerScan | null;
  selected: Result | null;
  provider: Provider;
  detail: Detail | null;
  pendingArtworkSave: PendingArtworkSave | null;
  artworkBusy: boolean;
  artworkError: string;
  artworkMessage: string;
  pendingLocalPoster: PendingLocalPoster | null;
  localPosterBusy: boolean;
  localPosterError: string;
  localPosterMessage: string;
}

export interface ArtworkSaveActions {
  openDialog(options: Record<string, unknown>): Promise<string | string[] | null>;
  selectedPathsInOrganizer(): string[];
  chooseMovieLocalPoster(mediaPath: string, artworkPath: string): void;
  loadLocalArtworkPreviews(images: LocalArtworkFile[]): Promise<void>;
}

export interface ArtworkSaveDeps {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  isDesktop: () => boolean;
  host: ArtworkSaveHost;
  actions: ArtworkSaveActions;
}

export function createArtworkSave(deps: ArtworkSaveDeps) {
  const { host: h, actions, invoke, isDesktop } = deps;
  const { openDialog, selectedPathsInOrganizer, chooseMovieLocalPoster, loadLocalArtworkPreviews } = actions;

  function stageArtworkSave(
    candidate: Artwork | undefined,
    targets: ArtworkTarget[],
    label: string,
    queueBindings: (ArtworkQueueBinding | null)[] = [],
  ) {
    if (!candidate || !h.organizerScan || !targets.length) return;
    h.pendingArtworkSave = { candidate, targets, label, queueBindings, saveBesideMedia: h.savePosterBesideMedia };
    h.artworkError = "";
    h.artworkMessage = "";
  }

  function plannedArtworkPaths(): string[] {
    if (!h.pendingArtworkSave) return [];
    if (!h.pendingArtworkSave.saveBesideMedia) return ["Private application artwork cache", "Poster source and license details saved with the cached image"];
    const extension = artworkFileExtension(h.pendingArtworkSave.candidate);
    return h.pendingArtworkSave.targets.map((target) => `${target.directory.replace(/[\\/]$/, "")}/${target.basename}.${extension}`);
  }

  async function applyArtworkSave() {
    if (!isDesktop() || !h.organizerScan || !h.pendingArtworkSave) return;
    const pending = h.pendingArtworkSave;
    h.artworkBusy = true;
    h.artworkError = "";
    h.artworkMessage = "";
    try {
      const result = pending.saveBesideMedia
        ? await invoke<ArtworkSaveResult>("save_library_artwork", {
          request: { root: h.organizerScan.root, candidate: pending.candidate, targets: pending.targets },
        })
        : await invoke<ArtworkSaveResult>("cache_library_artwork", { candidate: pending.candidate });
      let nextMovieAssignments = h.movieAssignments;
      let nextSeriesAssignments = h.seriesAssignments;
      let assignedPosterCount = 0;
      for (const [index, binding] of pending.queueBindings.entries()) {
        const savedPath = (pending.saveBesideMedia ? result.files[index] : result.files[0])?.path;
        if (!binding || !savedPath) continue;
        if (binding.kind === "movie") {
          const review = nextMovieAssignments[binding.mediaPath];
          if (review?.posterId === binding.candidateId) {
            nextMovieAssignments = {
              ...nextMovieAssignments,
              [binding.mediaPath]: { ...review, localPosterPath: savedPath },
            };
            assignedPosterCount += 1;
          }
        } else {
          const review = nextSeriesAssignments[binding.seriesRoot];
          if (binding.kind === "series") {
            if (review?.posterId !== binding.candidateId) continue;
            nextSeriesAssignments = {
              ...nextSeriesAssignments,
              [binding.seriesRoot]: { ...review, localPosterPath: savedPath },
            };
            assignedPosterCount += 1;
          } else {
            if (h.selectedArtwork[`season:${binding.seasonNumber}`] !== binding.candidateId || !review) continue;
            nextSeriesAssignments = {
              ...nextSeriesAssignments,
              [binding.seriesRoot]: {
                ...review,
                localSeasonPosterPaths: { ...review.localSeasonPosterPaths, [String(binding.seasonNumber)]: savedPath },
              },
            };
            assignedPosterCount += 1;
          }
        }
      }
      h.movieAssignments = nextMovieAssignments;
      h.seriesAssignments = nextSeriesAssignments;
      h.artworkMessage = `${result.message} Saved ${result.files.length} image${result.files.length === 1 ? "" : "s"}.${assignedPosterCount ? ` ${assignedPosterCount} reviewed poster selection${assignedPosterCount === 1 ? " is" : "s are"} ready for the next MKV output.` : " The saved image was not attached to this review because its selected match changed while the download was in progress. Choose it as a local poster before creating an MKV."}`;
      h.pendingArtworkSave = null;
    } catch (cause) {
      h.artworkError = String(cause);
    } finally {
      h.artworkBusy = false;
    }
  }

  async function chooseLocalPoster(file: OrganizerFile) {
    if (!isDesktop() || !h.organizerScan) return;
    const path = await openDialog({
      directory: false,
      multiple: false,
      title: `Choose poster for ${file.name}`,
      filters: [{ name: "Poster image", extensions: ["jpg", "jpeg", "png", "webp"] }],
    });
    if (!path || Array.isArray(path)) return;
    const extension = path.replaceAll("\\", "/").split("/").at(-1)?.split(".").at(-1)?.toLowerCase() ?? "";
    if (!["jpg", "jpeg", "png", "webp"].includes(extension)) {
      h.localPosterError = "Choose a JPEG, PNG, or WebP image.";
      return;
    }
    const target = movieArtworkTarget(file);
    h.pendingLocalPoster = {
      sourcePath: path,
      target,
      targetPath: h.savePosterBesideMedia ? joinPath(target.directory, `${target.basename}.${extension === "jpeg" ? "jpg" : extension}`) : "Private application artwork cache",
      mediaPath: file.path,
      saveBesideMedia: h.savePosterBesideMedia,
    };
    h.localPosterError = "";
    h.localPosterMessage = "";
  }

  async function saveLocalPoster() {
    if (!isDesktop() || !h.organizerScan || !h.pendingLocalPoster) return;
    h.localPosterBusy = true;
    h.localPosterError = "";
    try {
      const result = h.pendingLocalPoster.saveBesideMedia
        ? await invoke<{ files: string[]; message: string }>("save_local_library_artwork", {
          request: { root: h.organizerScan.root, sourcePath: h.pendingLocalPoster.sourcePath, targets: [h.pendingLocalPoster.target] },
        })
        : await invoke<{ files: string[]; message: string }>("cache_local_library_artwork", { sourcePath: h.pendingLocalPoster.sourcePath });
      if (h.pendingLocalPoster.mediaPath && result.files[0]) {
        chooseMovieLocalPoster(h.pendingLocalPoster.mediaPath, result.files[0]);
        await loadLocalArtworkPreviews([{ path: result.files[0], name: basename(result.files[0]), role: "Selected poster" }]);
      }
      h.localPosterMessage = `${result.message} Saved to ${result.files.join(", ")}`;
      h.pendingLocalPoster = null;
    } catch (cause) {
      h.localPosterError = String(cause);
    } finally {
      h.localPosterBusy = false;
    }
  }

  function movieArtworkTargets(): ArtworkTarget[] {
    const image = selectedMovieArtwork();
    const selectedId = h.selected ? `${h.provider}:${h.selected.id}` : undefined;
    const currentScan = h.organizerScan;
    if (!image || !currentScan || !selectedId) return [];
    return selectedPathsInOrganizer().flatMap((path) => {
      const assignment = h.movieAssignments[path];
      const file = currentScan.files.find((item) => item.path === path);
      if (!assignment || !file || assignment.recordId !== selectedId
        || !assignment.artworkCandidates.some((candidate) => candidate.id === image.id)) return [];
      return [movieArtworkTarget(file)];
    });
  }

  function movieArtworkBindings(image: Artwork): Extract<ArtworkQueueBinding, { kind: "movie" }>[] {
    const selectedId = h.selected ? `${h.provider}:${h.selected.id}` : undefined;
    const currentScan = h.organizerScan;
    if (!currentScan || !selectedId) return [];
    return selectedPathsInOrganizer().flatMap((path) => {
      const assignment = h.movieAssignments[path];
      const file = currentScan.files.find((item) => item.path === path);
      if (!assignment || !file || assignment.recordId !== selectedId
        || !assignment.artworkCandidates.some((candidate) => candidate.id === image.id)) return [];
      return [{ kind: "movie" as const, mediaPath: path, candidateId: image.id }];
    });
  }

  function assignSelectedPosterToMatchingCopies() {
    const image = selectedMovieArtwork();
    const selectedId = h.selected ? `${h.provider}:${h.selected.id}` : undefined;
    if (!image || !selectedId) return;
    const next = { ...h.movieAssignments };
    for (const path of selectedPathsInOrganizer()) {
      const assignment = next[path];
      if (assignment?.recordId === selectedId
        && assignment.artworkCandidates.some((candidate) => candidate.id === image.id)) {
        next[path] = { ...assignment, posterId: image.id, localPosterPath: "" };
      }
    }
    h.movieAssignments = next;
  }

  function stageCurrentFilmPoster() {
    const image = selectedMovieArtwork();
    if (!image) return;
    assignSelectedPosterToMatchingCopies();
    const currentScan = h.organizerScan;
    const bindings = movieArtworkBindings(image);
    if (!currentScan) return;
    const destinations = bindings.flatMap((binding) => {
      const file = currentScan.files.find((item) => item.path === binding.mediaPath);
      return file ? [{ target: movieArtworkTarget(file), binding }] : [];
    });
    stageArtworkSave(image, destinations.map(({ target }) => target), "poster for selected copies assigned to this film", destinations.map(({ binding }) => binding));
  }

  function selectedSeasonArtwork(seasonNumber: number | null): Artwork | undefined {
    if (seasonNumber === null || !h.detail) return undefined;
    const selectedId = h.selectedArtwork[`season:${seasonNumber}`];
    return h.detail.seasons.find((season) => season.number === seasonNumber)?.artwork.find((image) => image.id === selectedId);
  }

  function selectedMovieArtwork(): Artwork | undefined {
    return h.detail?.artwork.find((image) => image.id === h.selectedArtwork.movie);
  }

  function selectedSeriesArtwork(): Artwork | undefined {
    return h.detail?.artwork.find((image) => image.id === h.selectedArtwork.series);
  }

  return { stageArtworkSave, plannedArtworkPaths, applyArtworkSave, chooseLocalPoster, saveLocalPoster, movieArtworkTargets, movieArtworkBindings, assignSelectedPosterToMatchingCopies, stageCurrentFilmPoster, selectedSeasonArtwork, selectedMovieArtwork, selectedSeriesArtwork };
}
