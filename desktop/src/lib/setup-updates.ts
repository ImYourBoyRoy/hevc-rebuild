// ./desktop/src/lib/setup-updates.ts
// Turns the one-pass setup plan into the header notice for missing or out-of-date tools, and
// names how often the app re-checks. Pure: no native calls, so it is unit-tested in node.

import type { ProvisionPlan } from "./types.js";

/** Tools are re-checked this often while the app is open (and once at launch). */
export const SETUP_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;

export interface SetupUpdateSummary {
  count: number;
  label: string;
  detail: string;
}

/** A stale catalog alone is not an update: only missing packages, package updates and helper work count. */
export function setupUpdateSummary(plan: ProvisionPlan | null): SetupUpdateSummary | null {
  if (!plan) return null;
  const helperWork = plan.helper.action === "install" || plan.helper.action === "update";
  const count = plan.install_packages.length + plan.upgrade_packages.length + (helperWork ? 1 : 0);
  if (!count) return null;
  const parts: string[] = [];
  if (plan.install_packages.length) parts.push(`Install missing: ${plan.install_packages.join(", ")}`);
  if (plan.upgrade_packages.length) parts.push(`Update: ${plan.upgrade_packages.join(", ")}`);
  if (plan.helper.action === "install") parts.push("Install the HDR10+ helper");
  if (plan.helper.action === "update") parts.push(`Update the HDR10+ helper ${plan.helper.installed} to ${plan.helper.latest}`);
  return { count, label: `${count} ${count === 1 ? "UPDATE" : "UPDATES"}`, detail: `${parts.join(". ")}. Select to review and install in one step.` };
}
