import fs from "node:fs";

const [manifestPath, releasePath] = process.argv.slice(2);
if (!manifestPath || !releasePath) {
  console.error("Usage: node tools/verify-release-manifest.mjs latest.json release.json");
  process.exit(2);
}

const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
const release = JSON.parse(fs.readFileSync(releasePath, "utf8"));
const assets = new Set((release.assets ?? []).map((asset) => asset.name));
const requiredPlatforms = ["windows-x86_64", "darwin-aarch64", "darwin-x86_64", "linux-x86_64"];
const missing = [];

if (!manifest.version || typeof manifest.version !== "string") missing.push("manifest.version");
if (!manifest.platforms || typeof manifest.platforms !== "object") missing.push("manifest.platforms");

for (const platform of requiredPlatforms) {
  const entry = manifest.platforms?.[platform];
  if (!entry) {
    missing.push(`platform:${platform}`);
    continue;
  }
  if (typeof entry.signature !== "string" || entry.signature.length < 20) missing.push(`signature:${platform}`);
  if (typeof entry.url !== "string" || !entry.url.startsWith("https://")) {
    missing.push(`url:${platform}`);
    continue;
  }
  const assetName = decodeURIComponent(new URL(entry.url).pathname.split("/").at(-1) ?? "");
  if (!assets.has(assetName)) missing.push(`asset:${assetName}`);
}

const msiAssets = [...assets].filter((asset) => asset.toLowerCase().endsWith(".msi"));
if (msiAssets.length) missing.push(`forbidden-msi:${msiAssets.join(",")}`);

if (missing.length) {
  console.error(`Release manifest verification failed: ${missing.join("; ")}`);
  process.exit(1);
}

console.log(`Release manifest verified for ${requiredPlatforms.length} signed platforms; no MSI assets present.`);
