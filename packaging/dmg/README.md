# Direction A: Finder welcome window

Approved direction: dark surround, two light wells, title and install instruction.
The PNG contains no app icon, Applications icon, or filename label. Finder draws
those from the actual files. Its labels remain black with picture backgrounds.

Run `python3 packaging/dmg/restyle.py IMAGE APP_FILENAME BACKGROUND` after creating
the image and before signing/notarizing it. The helper converts a private temporary
copy, mounts it, asks Finder to save the layout, detaches, compresses, verifies,
and atomically replaces the image. App contents are not edited. No extra Python
packages, downloaded executables, runner jobs, cache or artifact retention.

`welcome*.png` is 4096 x 2160 at 72 dpi. The approved 775-point composition is
anchored top-left; extra space is flat #171a20. Standard initial Finder bounds are 775 x 540
including window chrome. Icon positions are (199,247) and (576,247), size 128,
label size 16. The bottom 59 points of the 480-point composition are expendable.
A matching background colour is set before the picture, but Finder may ignore
that fallback. Resize coverage beyond the 4096-point canvas is not promised.

## Native acceptance (required before calling the look verified)

1. Build a DMG through the existing macOS packaging path. Confirm signing and
   notarization checks still run after restyling. Mount the final read-only DMG.
2. Inspect the real Finder pixels: title, real app/Applications icons, black
   filename labels inside wells, no overlapping extra files, and footer visible.
3. Enlarge the window. Confirm top-left artwork stays fixed and the dark canvas
   covers the enlargement. Also shrink it and test path/status bars both ways.
4. Test scrollbars set to Always and hidden files both off/on. Determine whether
   the oversized background alone adds scroll extents; do not infer this from an
   ordinary smaller-background screenshot. Record macOS version and screenshots.
5. Verify normal drag-to-Applications installation. Do not mistake PNG previews,
   mock tests, hdiutil verification, or saved DS_Store existence for visual proof.

This change does not alter any workflow trigger, runner class, architecture,
cache/retention policy, publish destination or signing/notarization authority.
Dayflow's own installer is intentionally unchanged.

Vahti opts into a mounted read-only Finder capture in its existing macOS package
step. `DMG_WELCOME_CAPTURE_LOG=1` emits only the DMG window's PNG as base64 between
`DMG_WELCOME_PNG_BEGIN/END` markers in the existing run log, initial and enlarged.
There is no artifact upload/retention or extra runner job. It never screenshots
the desktop. `DMG_WELCOME_CAPTURE_UNAVAILABLE` means pixels remain unverified.
A successful screenshot still needs human inspection; saved-layout checks alone
are not visual acceptance. This opt-in is ignored outside GitHub Actions.

Artwork source: `render_backgrounds.py`, Pillow and DejaVu Sans (development-only).
The committed PNGs require no image library at packaging time. The standard layout expects only the app and Applications at the visible root.
Solmu additionally preserves its shipped notices in a visible Documentation folder
and adds a third light well below the install pair, with a taller initial window.
Unexpected extra files fail closed rather than hiding their content or leaving
black labels on dark art.

The owner requested Finnish + English together on 11 November 2026.
Finnish instructions use natural imperative wording; key senses checked against
https://www.kielitoimistonsanakirja.fi/asentaa and
https://www.kielitoimistonsanakirja.fi/vet%C3%A4%C3%A4 and
https://www.kielitoimistonsanakirja.fi/kansio .
The dictionary grounds word senses, not a claim that it approved these sentences.
