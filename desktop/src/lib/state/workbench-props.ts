// ./desktop/src/lib/state/workbench-props.ts
// The part of the metadata workbench's props that its derived values and effects read or write. The component
// builds one object with getters (and setters for the bindable props) over its `$props()` variables, so
// reads inside an effect stay reactive.

import type { ScanResult } from "../types.js";
import type { MovieAssignment, SeriesAssignment } from "../metadata-workbench-types.js";

export interface WorkbenchPropsView {
  readonly scanResult: ScanResult | null;
  readonly focusMediaPath: string | null;
  readonly compactItemMode: boolean;
  readonly workbenchActive: boolean;
  readonly autoLookupPending: boolean;
  readonly metadataAutoLookup: boolean;
  selectedMediaPaths: string[];
  movieAssignments: Record<string, MovieAssignment>;
  seriesAssignments: Record<string, SeriesAssignment>;
  onAutoLookupConsumed(): void;
  onClaimAutoLookup(): boolean;
}
