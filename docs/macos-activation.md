# macOS activation

The delegate handlers use architecture-correct fallback encodings when protocol
metadata is unavailable. This path is active by default and covered by native
registration tests.

Direct native restoration of minimized Solmu windows is on by default. The owner
reported that Cmd-Tab restoration worked with the opt-in in manual.15, and that
closed-window activation worked without it. That is one real-device acceptance
report, not a guarantee across all macOS/window configurations.

To opt out, launch the app process with
`SOLMU_EXPERIMENTAL_RESTORE_MINIMIZED=0`. Any other value, or no variable, keeps
restoration on. The existing variable name stays for compatibility. Finder
launches do not normally inherit terminal environment variables.

This restores every minimized Solmu window, not only the main window. Dock reopen
stays separate, and closed-window activation still uses the existing resident
window/event path. No profile or message data is changed.
