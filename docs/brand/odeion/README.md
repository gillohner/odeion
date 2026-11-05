# Odeion mark (groove-O)

The approved Odeion logo: an O cut into three sound grooves, one colour (`#a496ff`) on the app's
dark background (`#0f1014`), no gradients. Approved by the project owner on 2026-11-03; the
designs are supplied as finished files, not generated here.

| File | What it is |
| :-- | :-- |
| `odeion-mark.svg` | The mark alone, `currentColor` on transparency (the sidebar mark in the web app uses the same three paths) |
| `odeion-app-icon.svg` | The mark on a full dark square: the web app's `icon.svg` |
| `odeion-logo-preview.png` | The approved preview: the mark, the word mark and small sizes |
| `sources.json` | SHA-256 of every source, and of each file installed in `crates/pm-api/static/` with the source it is a byte copy of |

The PNG icons in `crates/pm-api/static/` (`apple-touch-icon.png` 180, `icon-192.png`,
`icon-512.png`, `icon-maskable-512.png`) are the supplied renders, copied byte for byte. The
whole mark lies within radius 157 of the 512 px canvas, inside the maskable safe zone (radius
204.8), so the same full-bleed render serves as the maskable icon. `tools/pwa-check.js` checks
every hash, size, the safe zone and the sidebar mark's paths.

To change the logo, replace these sources and the installed files together and update
`sources.json`; nothing regenerates them.
