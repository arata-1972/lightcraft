# Asset attribution

Every asset shipped in this repository is listed here (see the asset rules in `AGENTS.md`). No asset may come from
Adobe products. Add an entry in the same commit as the asset.
`cargo xtask assets` (part of `cargo xtask ci`) fails if any image, icon, font, sound, video, raw file or colour profile
in the repository is not matched by a path pattern in the first column (`*` = any characters except `/`; a trailing `/`
covers a whole directory), or if a licence file referenced in the Licence column is missing.

| Path | Asset | Creator | Source | Licence | Added | Modifications |
|---|---|---|---|---|---|---|
| `assets/fonts/Inter-*.ttf` (Regular, Medium, SemiBold) | Inter | Rasmus Andersson | https://github.com/rsms/inter | SIL Open Font License 1.1 (`assets/fonts/OFL-Inter.txt`) | 2026-09-30 | none |
| `assets/fonts/BIZUDMincho-Regular.ttf` | BIZ UDMincho Regular | The BIZ UDMincho Project Authors (Morisawa Inc., design TypeBank Co., Ltd.) | https://github.com/google/fonts/tree/63833b7d10bb1f86a8f0b91cba2d3ae1f68d1aa3/ofl/bizudmincho | OFL-1.1 (`assets/fonts/OFL-BIZUDMincho.txt`) | 2026-10-05 | none |
| `assets/fonts/OFL-BIZUDMincho.txt` | BIZ UDMincho licence | The BIZ UDMincho Project Authors | https://github.com/google/fonts/blob/63833b7d10bb1f86a8f0b91cba2d3ae1f68d1aa3/ofl/bizudmincho/OFL.txt | OFL-1.1 | 2026-10-05 | none |
| `assets/app-icon/` (`lightcraft.svg` and the PNG/ICNS/ICO/SVG files derived from it) | LightCraft app icon (lynx) | LightCraft project owner (drawn in ArtCraft) | original work (vectorised from the owner's drawing; see `assets/app-icon/README.md`) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | 2026-10-02 | vectorised to SVG; rendered to PNG/ICNS/ICO by `packaging/icons.sh` |
| `crates/scenes/` (generated images) | Procedural demo photographs | LightCraft contributors | original work (generated at runtime, no source imagery) | MIT OR Apache-2.0 | 2026-09-30 | n/a |
| `crates/ui-egui/src/icons.rs` | UI icons drawn as vector paths in code | LightCraft contributors | original work | MIT OR Apache-2.0 | 2026-09-30 | n/a |
| `docs/images/*-tetons.jpg`, `docs/images/grid-pd.jpg` (screenshots containing it) | "The Tetons and the Snake River" | Ansel Adams (1942), U.S. National Archives | https://commons.wikimedia.org/wiki/File:Adams_The_Tetons_and_the_Snake_River.jpg | Public domain (U.S. federal government work) | 2026-09-30 | developed in LightCraft; shown inside app screenshots |
| `docs/images/*-migrant-mother.jpg`, `docs/images/grid-pd.jpg` | "Migrant Mother" | Dorothea Lange (1936), Farm Security Administration / Library of Congress | https://commons.wikimedia.org/wiki/File:Lange-MigrantMother02.jpg | Public domain (U.S. federal government work) | 2026-09-30 | developed in LightCraft; shown inside app screenshots |
| `docs/images/*-earthrise.jpg`, `docs/images/grid-pd.jpg` | "Earthrise" (AS08-14-2383) | NASA / Bill Anders, Apollo 8 (1968) | https://commons.wikimedia.org/wiki/File:NASA-Apollo8-Dec24-Earthrise.jpg | Public domain (NASA) | 2026-09-30 | developed in LightCraft; shown inside app screenshots |
| `docs/images/*-blue-marble.jpg`, `docs/images/grid-pd.jpg` | "The Blue Marble" (AS17-148-22727) | NASA, Apollo 17 crew (1972) | https://commons.wikimedia.org/wiki/File:The_Earth_seen_from_Apollo_17.jpg | Public domain (NASA) | 2026-09-30 | developed in LightCraft; shown inside app screenshots |
| `docs/images/*.jpg` (UI) | LightCraft application screenshots | LightCraft contributors | original work (captured with `docs/showcase/`) | MIT OR Apache-2.0 | 2026-09-30 | n/a |
| `docs/brand/` (artcraft-logo and artcraft-mark, SVG and PNG) | ArtCraft wordmark and mark | ArtCraft Team | original work (https://getartcraft.com/) | ArtCraft trademark, see `docs/brand/LICENSE-brand.txt` (not open source) | 2026-10-01 | none |

| `assets/fonts/BIZUDPGothic-*.ttf` | BIZ UDPGothic (Regular, Bold) | Morisawa / BIZ UDGothic Project Authors | https://github.com/googlefonts/morisawa-biz-ud-gothic/tree/18934af56b9c003ca58c54bffbf226848cb11032 | SIL Open Font License 1.1 (`assets/fonts/OFL-BIZUDGothic.txt`) | 2026-10-05 | none |

Notes
- Fonts authored or published by Adobe (Source Sans/Serif/Code, Source Han, …) are not used, even though some are
  OFL-licensed: the asset rule excludes anything from Adobe. Source Sans 3 was removed on 2026-09-30 and replaced by Inter.
