// ./desktop/src/lib/app-version.ts
// The application version for display, read from package.json at build time so the sidebar can
// never drift from the release number.
import pkg from "../../package.json";

export const APP_VERSION: string = pkg.version;
