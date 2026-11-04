# One design for phone, tablet and desktop (design note, step 1)

**Since 2026-11-04 there are two layouts, not three** (owner: "same features and design as desktop … 2 sizes basically but responsive"): **phone** under 600 px and **desktop** from 600 px, tablets included. See "Tablets: the desktop layout" below; the tablet rail drawn in the 10-26 note is gone.

STEWARD-UX-RESPONSIVE.md step 1. **Approved by the owner on 2026-10-26, as drawn.** The same note as a page, with the wireframes scaled side by side: `responsive/note.html` (published for the owner; link in ~/QUESTIONS.md). The study behind it: `mobile-study.md`.

## Rules
- **Screens don't scroll; lists do.** Now playing and the zone picker fit the screen. Library, album, artist, playlist and search scroll; their headers fold into a compact bar.
- **Secondary things are sheets.** Queue, Lyrics, Details (signal path, file, credits), Sources (also on) and Tags open over Now playing: a bottom sheet on a phone, a docked panel beside it in the desktop layout (tablets too).
- **Thumbs first.** 40–44 px targets on any touch screen (`pointer: coarse`), whatever the layout; transport and the secondary row in the bottom half; swipe a row for Play next, long-press for its menu.
- **Same parts at every width;** only the arrangement and density change. Safe areas, both themes, reduced motion, font scale 1.3.

## Phone (under 600 px)
| Library, mini player, tab bar | Now playing, one screen | A sheet over Now playing | Settings, four groups |
| :-: | :-: | :-: | :-: |
| ![](responsive/phone-library.png) | ![](responsive/phone-now-playing.png) | ![](responsive/phone-queue-sheet.png) | ![](responsive/phone-settings.png) |

- **Tabs:** Home, Search, Library, Zones. The mini player above them carries the quality and the zone.
- **Library:** my library first; shared libraries and Downloads as their own sections.
- **Now playing:** art; title with the heart; the quality label (opens Details, the signal path); scrubber; transport. One row opens the five sheets; swipe down collapses it.
- **Settings:** four groups, by where each setting is saved (STEWARD-SETTINGS-GROUPS.md). Each group header says where in one line:
  - **You:** saved on your homeserver, so it follows you to every node and device. My accounts, My library and sharing, Home, layout and appearance, playback quality.
  - **This phone:** only on this device. The USB DAC and bit-perfect, Downloads and cache.
  - **This node (admin):** for everyone on this node. Service apps, People, Library folders, zones and outputs, Remote access and updates.
  - **This homeserver (admin, only when the node runs its own):** sign-ups and invites, users, quotas, storage, domain.
  - Every setting sits in exactly one group. Where a node default and your own choice both exist (playback quality), the node group holds the default and You holds the override; each row says which applies. In the desktop layout (tablets too) the groups are the left pane.

## Tablets: the desktop layout (600 px and up; owner, 2026-11-04)
Tablets get the desktop layout: the sidebar with every place, the desktop player bar, dense lists, the docked panel and the desktop Now playing. Same features and design; it only reflows to fit:
- **Sidebar:** icons with their names as tooltips under 1100 px (☰ expands it to names); every place stays, and it scrolls when the window is short.
- **Player bar:** its three groups shrink and never overlap. The seek bar takes the free width; the sound pill is its source mark, dot and quality badge under 1100 px (the zone button names the device); the zone's name ellipsizes. Under 1280 px the volume slider is a button that opens the Sound panel at it. Under 900 px shuffle and repeat move to Now playing and the zone button drops its chevron; under 760 px the volume button goes too (the volume stays in the Sound panel and in Now playing); under 680 px the seek bar drops its times and the pill its source mark.
- **Hero:** the cover stays beside the words down to 600 px (140 px under 720 px). "Play on…" is its icon under 900 px, and a playlist's Delete is in its ⋯ menu up to 1024 px, so the buttons stay on one row.
- **Touch:** on a touch screen (`pointer: coarse`) the targets stay 40 px, song rows show the heart (when on) and ⋯ without hover, and the whole player bar opens Now playing, as on a phone (with a mouse its title and artist are links).
- **Installed (standalone):** the top bar, the sidebar and Now playing start below the status bar (`env(safe-area-inset-top)`); on a touch screen the sidebar also keeps its top corner free for iPadOS window controls (Stage Manager, windowed apps).
- **Checked by** `tools/ui-tablet.js`: 600, 700, 820, 1024, 1180 and 1366 px, portrait and landscape, both themes, plus 390 px as the phone.

The 10-26 tablet drawing (`responsive/tablet-now-playing.png`, a rail) is kept for the record only.

## Desktop (600 px and up)
![](responsive/desktop-album.png)

- **Arrangement:** a sidebar (with a "Shared with you" group), dense lists with columns (title, artist, quality, time), the queue/lyrics/details panel docked right, and the player bar along the bottom.
- **Now playing:** a full view with the panel beside the art.

## Colour from where it plays (§6)
| Spotify | TIDAL | Qobuz |
| :-: | :-: | :-: |
| ![](responsive/colour-spotify.png) | ![](responsive/colour-tidal.png) | ![](responsive/colour-qobuz.png) |

- **The accent:** while a song plays, the player surfaces take one accent from where it streams: the quality label, the scrubber, the source chip and the play button. Local files and the homeserver keep ours.
- **The cover:** Now playing gets a soft gradient from it.
- **A tint only:** no logos beyond the small source mark, no copied layouts or fonts. Lists keep the app's own theme.
- **Contrast:** each accent is checked in both themes and nudged where it fails: text on the background at least 4.5:1, the button's ink on the accent at least 4.5:1.

  | Source | Light | Dark | Dark ink |
  | :-- | :-- | :-- | :-- |
  | Spotify | `#13803B` | `#1ED760` | yes |
  | TIDAL | `#007C89` | `#33E8E0` | yes |
  | Qobuz | `#1F5BD6` | `#6E9BFF` | yes |
  | ours | `#5b45d6` | `#a596ff` | yes |

  Each source declares its accent next to its features when this is built.
- **Settings › Appearance:** Colour from source (on), Colour from album art (on), or neither. With reduced motion the colour switches without a fade.

## Layout option
Settings › Home and layout:
- **Layout:** Automatic (phone under 600 px, desktop from there), Phone or Desktop (either at any width). The old Compact becomes Phone; Comfortable (the rail from 600 px, a phone below) becomes Automatic.
- **Dense lists**, in the desktop layout.
- **Home rows.**

Per person, saved on their homeserver (STEWARD-MULTI-USER.md).

## The owner's decisions (2026-10-26)
1. **The fourth tab:** Zones.
2. **Swipes on rows:** right = Play next, left = Add to playlist; long-press opens the row's menu.
3. **Lyrics:** in the sheet with the others.

## Build order after the look
Each step is one PR:
1. the shell;
2. Now playing and its sheets;
3. lists;
4. Settings.

Each step gets screenshots at 390, 820 and 1440 px in both themes, and runs the overlap, accessibility and click-every-button checks at all three widths.
