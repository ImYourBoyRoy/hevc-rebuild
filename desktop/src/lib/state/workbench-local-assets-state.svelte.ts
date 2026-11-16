// ./desktop/src/lib/state/workbench-local-assets-state.svelte.ts
// Metadata workbench local artwork: the scanned assets, scan error and generation, and the poster preview cache with its errors and busy flags.
// A plain rune class: the workbench component creates one instance, reads and writes its fields directly and
// passes the instance to the controllers extracted from the component as their typed host.
import type { LocalAssetsScan } from "../metadata-workbench-types.js";

export class WorkbenchLocalAssetsState {
  localAssets = $state<LocalAssetsScan | null>(null);
  localAssetScanError = $state("");
  localAssetGeneration = 0;
  localArtworkPreviews = $state<Record<string, string>>({});
  localArtworkPreviewErrors = $state<Record<string, string>>({});
  localArtworkPreviewBusy = $state<Record<string, boolean>>({});
}
