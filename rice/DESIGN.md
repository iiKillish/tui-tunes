# UI design (ratatui rewrite)

Chosen from the layout mockups:

- Layout B: a now-playing **hero** at the top of the portrait screen (left column on wide
  windows): big centered cover, title + ♥, artist, album, then **K3 phone-style controls**: the
  progress bar first (elapsed / remaining under or beside it), then shuffle at the left edge,
  previous / play-pause / next in the middle, repeat at the right edge; volume under them.
  Falls back to a 5-row bottom bar on small windows.
- Under the hero: **T1 tabs**: plain words (Home, Search, Library, Up next, Lyrics), the current
  one bold in the accent and underlined; no fills.
- **V1 Clean**: no background fills or borders on the main screens (menus/dialogs may use a
  rounded border). Section titles bold in the accent.
- **Home H3**: a featured "Pick up where you left off" card (cover, title, subtitle, ▶ Play), filter
  pills (All · Albums · Playlists · Artists · Songs; current one accent), then one list of
  rows (2-row cover + title + "Album · Artist").
- **Pages P1+P2**: header with the cover on the left, kind / bold title / details, buttons
  (▶ Play filled in the accent, ⇄ Shuffle, ♡ Like); the header background is tinted with the
  cover's color fading into the page. Then numbered rows with the length on the right.
- **S2 selection**: the selected row gets a darker background (`statusbar_bg`) and a `▌` in the
  accent at its left edge (`Styles::selected` + `SELECTION_MARKER`/`Styles::marker`).
- **R2 rows**: number, small cover (1 row), title, artist, length.
- Covers via Kitty graphics (`tui::images`); Catppuccin colors from the theme (`theme.toml`).
