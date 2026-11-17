// ./desktop/src/lib/themes.ts
// Registry of the application themes. The single frontend source of truth for the theme ids, their
// picker labels and the DOM attributes each one publishes. Pure: no IPC, no DOM, no state. The Rust
// side validates the same ids in `app_preferences.rs` (`APP_THEMES`); keep the two in step.
// Tested by pnpm test:themes.
//
// Palette themes (cinema, oled, dos) publish the base family they extend as `data-theme`
// ("dark") so every existing dark/light rule keeps applying, and their own id as
// `data-palette`; the theme parts then override only the tokens that differ. Instrument skins keep
// publishing themselves as `data-theme` and `data-instrument`.

export type Theme = "light" | "dark" | "cinema" | "oled" | "dos" | "phosphor" | "ion" | "breach";
export type ThemeFamily = "light" | "dark" | "instrument";
export type ThemeScheme = "light" | "dark";

export interface ThemeDefinition {
  id: Theme;
  label: string;
  description: string;
  family: ThemeFamily;
  /** Base theme whose rules the palette extends, for palette themes. */
  base?: "light" | "dark";
  scheme: ThemeScheme;
}

export const themes: readonly ThemeDefinition[] = [
  { id: "light", label: "Light", description: "Warm paper surfaces with a forest sidebar.", family: "light", scheme: "light" },
  { id: "dark", label: "Dark", description: "Cool charcoal with a mint accent.", family: "dark", scheme: "dark" },
  { id: "cinema", label: "Cinema", description: "Warm amber on near-black, like a darkened theatre.", family: "dark", base: "dark", scheme: "dark" },
  { id: "oled", label: "OLED", description: "True black with quiet gray surfaces to save power on OLED screens.", family: "dark", base: "dark", scheme: "dark" },
  { id: "dos", label: "DOS", description: "A blue DOS text-mode interface: double-line frames, block shadows, VGA colours.", family: "dark", base: "dark", scheme: "dark" },
  { id: "phosphor", label: "Phosphor", description: "Green CRT terminal.", family: "instrument", scheme: "dark" },
  { id: "ion", label: "Ion", description: "Cold radar console.", family: "instrument", scheme: "dark" },
  { id: "breach", label: "Breach", description: "Hostile alarm console.", family: "instrument", scheme: "dark" },
];

export const themeOptions: readonly { value: Theme; label: string }[] = themes.map((theme) => ({ value: theme.id, label: theme.label }));

export interface ThemeAttributes {
  /** Value for `data-theme`: the base family for palette themes, the id otherwise. */
  theme: string;
  /** Value for `data-palette`: always the exact theme id. */
  palette: string;
  /** Value for `data-instrument`: only the instrument skins. */
  instrument: string | undefined;
}

export function themeAttributes(id: Theme): ThemeAttributes {
  const definition = themes.find((theme) => theme.id === id);
  const instrument = definition?.family === "instrument" ? id : undefined;
  return { theme: definition?.base ?? id, palette: id, instrument };
}

export function isTheme(value: unknown): value is Theme {
  return themes.some((theme) => theme.id === value);
}
