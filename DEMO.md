# Developing with the demo

The `demo` feature builds Solmu with offline sample chats, for development,
screenshots, and recorded tours. `AGENTS.md` describes the architecture and
the rules for changes; `CONTRIBUTING.md` lists the checks.

```sh
cargo run --features demo -- --demo            # sample chats, no connection
cargo run --features demo -- --demo-page login # or settings, pair, info, light, …
cargo run --features demo -- --demo-shot shot.png --demo-page chat,light
cargo run --features demo -- --demo-tour      # Space starts/replays a 41-second tour
cargo run --features demo -- --demo-tour --demo-tour-script whats-new # what 0.16 added
cargo run --features demo -- --demo-hover 900,400 # holds a fake pointer there
cargo run --features demo -- --demo --demo-titlebar # Windows/Linux: the drawn title bar
cargo test --all-features                      # includes a headless layout of every screen
cargo clippy --all-targets --all-features -- -D warnings
```

To include a default GIPHY key for GIF search, set it at build time. A key in
Settings overrides it:

```sh
SOLMU_GIPHY_KEY=your-key cargo build --release
```

The earlier `FASTSAPP_GIPHY_KEY` build variable remains supported as a fallback.

CI checks the complete lockfile against RustSec advisories with `cargo audit`.
Candidate-specific manual checks and results are tracked in the release PR.

## Recording a demo

The `demo` feature uses offline sample chats in a fresh temporary directory.
It does not open your linked account, read your message archive, connect to
WhatsApp, or register a tray icon. You can run it alongside your regular app.

```sh
cargo build --locked --features demo
./target/debug/solmu --demo-tour --demo-size 1280x800
```

The **Solmu Demo** window waits for **Space**. The 41-second tour starts with
search, switches chats with keyboard shortcuts, scrolls, right-clicks a message
and selects Reply, types quickly, completes emoji and mentions, searches the GIF
picker and sends a still sticker, opens group information and the shortcut list,
and changes themes through Settings. It uses the normal mouse and keyboard handlers;
a local responder handles outgoing messages with no WhatsApp connection.
The demo's profile pictures, photos, video clips, GIF-search thumbnails, and captioned stickers
are stock media compiled into demo builds only, all CC0 or in the public domain
(`assets/demo/SOURCES.md` lists each source); demo GIF search uses these local
fixtures, and the remaining stickers come from the bundled Noto emoji font. The tour makes no
sound and holds its final frame. Space rebuilds the sample and replays.
For an automatic start, add `--demo-tour-delay 5000` (milliseconds).
Use `--demo` instead of `--demo-tour` to explore the sample chats yourself.

`--demo-tour-script whats-new` plays an 86-second tour of what Solmu 0.16
added instead: the composer's plus menu and poll dialog, searching a chat and
narrowing it to a day, the photo preview, videos and round video messages
playing in place, sticker shelves and sticker search, message info in a group,
the Favorites and label chips and a chat's menu, recording a voice message and
choosing a playback speed, the chat list folded to avatars, hover controls,
Ctrl-click and Shift-click selection with Forward, and Settings (languages,
search, and the light theme). `--demo-tour-script launch` is the default.
Demo runs never open the microphone: recording plays back a synthetic tone.
Use `--demo-page rtl-self` for a self-chat of mixed Hebrew, Arabic, and
English lines.
Use `--demo-page composer-tools` to preview the WhatsApp-style composer pill
and its attachment and poll menu, or `--demo-page location` for the location
dialog with a spot pasted in. `typing`, `mention`, and
`emoji-complete` preview the multiline field and inline suggestions.
`mention-everyone` shows the group's synthetic `@all` message with the
everyone suggestion at the top of the composer list.
Use `--demo-page chat-menu` to preview the compact chat context menu,
`--demo-page chat-header-menu` for the menu at the top of an open chat, and
`--demo-page chat,voice,voice-menu` for a voice message's menu with its speeds.
`--demo-page motion` shows a motion photo, `motion-playing` plays its clip in
the bubble, `motion-preview` opens the photo in the full-window viewer, and
`motion-viewer` plays its clip there over the photo. `carousel-viewer` opens a
carousel card's picture in the viewer, among the card's other pictures.
`--demo-page private-reply-menu` shows a group message's menu with **Reply
privately**, `private-reply` the sender's chat with the group quote in the
composer, and `private-reply-locked-source` the same quote while its group is
locked.
`--demo-page call-bubbles` ends a group with one call entry of each kind
(answered with a length, unanswered, missed and declined, voice and video,
ours and members'), and `call-entries` files the sample calls in a chat.
`--demo-page video` shows a video and round video messages,
`--demo-page video-expanded` the video playing in the full-window viewer, and
`video-playing` or `note-playing` starts one of them, silently.
Locked chats preview with `channel`, `locked`, `locked-open` (the code is
`demo-code`), `locked-prompt`, `locked-setup`, and `keyring`; the app lock
with `app-lock` (the password is `demo-password`), `app-lock-wrong`,
`app-lock-forgot`, `app-lock-settings`, and `app-lock-setup`. `new-chat`,
`new-contact`, `unnamed-group`, and `react-picker` show those dialogs
(`new-contact-number` with a formatted number and no name); `new-group` New
chat's New group with two people picked and a search, `new-group-name` the
group being named, and `new-group-creating` it on its way; `group-info`,
`group-info-rename`, `group-info-saving`, and `group-info-locked` a group's
editable info; `meta-ai` a Meta AI reply with code and a table.
The editable group includes a multiline description; the locked group has none.
For deterministic theme screenshots, `--demo-page settings,omarchy` and
`--demo-page settings,omarchy-light` preview following dark and light Omarchy
palettes without changing the desktop theme.

Use `--demo-page preview` for a photo viewer with three pictures, text between
the second and third picture. Hover over the photo to show the previous/next
arrows on it, or use Left/Right arrow keys. Use `--demo-hover 590,450` to
include the arrows in a screenshot.

Use `--demo-page shared-contact` for an offline shared-contact card with synthetic
vCard data, or `--demo-page interactive` for text and button messages, or
`--demo-page interactive-media` for messages with an image, and
`--demo-page interactive-list` for a list message,
`--demo-page interactive-list-dialog` for its grouped choice dialog, `--demo-page carousel`
for a scrolling strip or `--demo-page carousel-pair` for two cards, and `--demo-page poll-empty`, `poll-voted`, or `poll-results`
for voting states. Use `--demo-page interactive-actions` for reply, list, copy-code, and unavailable
form actions. Add `,light` to
preview any of these in the light theme. Capture the app's own frame without desktop
content:

```sh
./target/debug/solmu --demo --demo-page interactive-media --demo-shot interactive.png
./target/debug/solmu --demo --demo-page interactive-media,light --demo-shot interactive-light.png
```

On Omarchy, run `omarchy screenrecord`, select the demo window, then press Space
in Solmu. Recording has no audio unless you explicitly enable desktop or
microphone audio. Stop with `omarchy screenrecord --stop-recording` after the
tour finishes. The default capture records a fixed rectangle, so keep the demo
window visible and stationary until recording stops.

To annotate the video with a visible pointer, click rings, and outlined shortcut
labels, add `--demo-tour-events tour.json` when launching the tour. After
recording, run:

```sh
python3 scripts/render-demo.py recording.mp4 tour.json launch.mp4 --start 0.8
```

Set `--start` to the recording time (in seconds) when you pressed Space. The
export trims the setup footage, adds a caption band below the app, and produces
a silent H.264 MP4. It requires `ffmpeg` with libass support and `ffprobe`.
`--scale 1.5` keeps 1.5 pixels per point, for example 1920 pixels across from a
1280-point window recorded at 2x; the default is one pixel per point.

These annotations are added during video export, not drawn by the app. The
trace contains only pointer coordinates and shortcut labels, not typed text.

Instead of recording the screen, the tour can save its own frames. With
`--demo-tour-frames DIR`, it starts at once, plays on a virtual clock (steady
frame times even when a frame is slow to draw), writes every frame as a PNG
at the window's pixel size, and quits when the tour ends. `--demo-fps` sets the
rate (30 by default). The window still has to be shown somewhere; a virtual
output keeps it off your screens. Then assemble and annotate the frames:

```sh
cargo build --release --locked --features demo
./target/release/solmu --demo-tour --demo-tour-script whats-new \
  --demo-size 1280x800 --demo-tour-frames frames --demo-tour-events tour.json
ffmpeg -framerate 30 -i frames/frame-%05d.png -c:v libx264 -crf 12 -pix_fmt yuv420p raw.mp4
python3 scripts/render-demo.py raw.mp4 tour.json whats-new.mp4 --scale 1.5
```

## Chat-list scope previews

`--demo-page scope-search` shows Unread with scoped search and an outside-scope hint; `scope-group` combines Unread and Groups. `grid` shows three synthetic incoming group messages on one bubble column. Add `,light` for the light palette and `--demo --demo-macos` for native Mac layout preview.

The four queue segments remain visible while typing. Filters composes category, favorites, labels, Unreplied and network/account choices rather than replacing the queue. All selects the whole current scope; Reset view removes the scope while preserving the query. The outside count covers available chat and backend message matches, not an estimated total across unloaded server pages. Unknown-chat and locked-chat message hits do not enter scoped results. Hide the chat list is in Filters and still uses Cmd+B (Ctrl+B off Mac).

`--demo-page day-separators` shows five synthetic calendar days: today, yesterday,
two days ago, three days ago and the previous year, for checking localized date labels.
Use `linked-day-separators` for the same dates in a linked-person timeline.

`--demo-page queue-counts` shows 12 available chats, 3 unread, 5 Reply later and 2 Read later. Add `,queue-counts-off` for the original icon-only control, `,light` for light theme, or `,settings-search=queue` for the preference. Counts are local queue items, not message totals.

`--demo-page active-avatar` opens the unread group in the collapsed rail to check its selected ring and unread badge. Add `,light` for the light theme; `--demo-hover 26,195` checks hover without moving the real pointer.

### Linked network chip lanes

`--demo-page linked-chip-lanes` starts a WhatsApp network run with an outgoing
message and a Signal run with an incoming message. Use `--demo-size 620x1000`
and `1180x1000`, with and without `,light`, to inspect the icon lanes and
wrapped message/footer layout. The runtime geometry regression also checks
both directions, themes and widths.

The chip-lanes fixture always puts WhatsApp before Signal, independent of the
launch hour. Signal timestamps are rebased after the fixed WhatsApp sample.

### Speaker-change streaks

`--demo-page group-stacks` shows synthetic three-speaker text streaks, a
returning speaker, reactions, and outgoing read receipts. Add `,light` for the
light theme; compare `--demo-size 1180x1100` and `--demo-size 620x1400`.
Each eligible pair overlaps by 6 logical points with its older lower edge in
front. Every bubble has a triangular top sender-side tail. Incoming group
shadows choose contrasting colours per sender streak, not a permanent colour
identity; names/avatars remain the identity cues. There are no accent rails or
stack separator borders. Reactions and time share a wrapping footer row.


`--demo-page card-widths` shows a file, voice note, reply and link preview at
one shared320pt content width, a reply and preview sharing the next400pt
step, plus plain text at80/160/240pt and longer text on the same320/400pt
grid. Both text and cards grow in80pt steps up to the viewport cap. Use `,light` and a narrow `--demo-size` to check viewport caps.
