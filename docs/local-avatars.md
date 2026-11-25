# Local avatars

Open Contact info or Group info, then choose **Customize avatar…**.
Choose a local image, or use up to three letters/numbers and a colour. The
editor previews the result. Save applies it to all of this identity's avatar
surfaces in Solmu, including favourites and the compact sidebar. Cancel or
Escape discards the draft. Reset to default removes the override and shows the
current source avatar again.

This changes only Solmu's display. It does not update a contact, Beeper account
or group profile photo. The existing group-photo change action is separate and
still changes the source group photo. Notification photos outside the app
continue to use source pictures.

A linked person's override covers its current member identities. Changing the
default sending bridge does not change the custom avatar. After unlinking, each
member keeps its local avatar until reset. Identity keys include the local
account and exact source/account/chat tuple, not the contact's name. Replacing a
source or switching accounts cannot inherit another account's override.

Images are limited to 10 MiB, 8192 pixels per side and 32 megapixels. Decode
allocation is bounded. Solmu applies image orientation, takes a square centre
crop and re-encodes a 512×512 PNG without source metadata. GIF uses its first
frame. Import and image writes run off the UI thread. Malformed or unsupported
images leave the previous avatar unchanged. The editor stays open if saving the
prepared image fails.

Overrides are stored with local settings. Prepared images are private files in
`custom-avatars` under Solmu's user-state directory, separate from the disposable
source-photo cache. Reset removes the override, not the user's original image;
previously prepared files may remain in that local directory. Nothing is
uploaded, and the feature has no network client, public database or API key.
