# Keychain access after updates

Solmu's Rust client, not its launcher, reads its saved Server token and local encrypted-draft key from the login Keychain. Those items use the services `rocks.zapfast.BeeperServer` and `rocks.zapfast.ZapFast`. Their names and encryption keys are unchanged.

The released manual.16 and manual.17 clients were Developer ID signed, but their nested executable signing identifiers contained different linker UUID suffixes. The designated requirement used by Keychain therefore changed on a rebuild. Packaging now gives the client the explicit identifier `fi.sulopuisto.fast-beeper.client` before release signing. The release signer preserves this identifier, and the mounted-DMG check rejects an unexpected identifier, team or build-pinned requirement. A macOS fixture builds two different clients through the real bundle path and verifies the identifier stays the same.

An old Keychain item may ask for approval once when moving from its old UUID-derived identity to the fixed one. The fix cannot silently rewrite the user's existing access-control list. For the expected Solmu item, Always Allow authorizes that application's future access. Do not delete encryption keys or allow every application access.

This does not add Touch ID to ordinary token/draft-key access. The separate app-lock item (`rocks.zapfast.AppLock`) already uses a biometric access-control flag. Changing existing encrypted storage to biometric-gated credentials needs a separate migration and recovery design.

No Keychain password or credential is read by the signing regression test. No signing secrets are needed for that fixture; final Developer ID requirement checks run only when verifying an actual release.
