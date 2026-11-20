# Local Server sign-in (0.20.2)

This source change adds an in-app sign-in window for an **already installed,
running local Beeper Server**. It does not install Server, create accounts,
copy a Desktop session, connect networks, reset keys, or make a remote endpoint
public. It is not a downloadable Mac release yet.

From the repository root, launch without a terminal token prompt:

```sh
uv run --no-project python contrib/beeper-tools/run_client.py --beeper-target server --sign-in
```

Or use the existing local Server root explicitly:

```sh
uv run --no-project python contrib/beeper-tools/run_client.py --base-url http://127.0.0.1:23374 --sign-in
```

For a compiled trial, append `--binary /actual/path/to/solmu`. Source launch
requires the normal build prerequisites. Read-only messaging remains the default;
sign-in and verification are separate, deliberate setup actions.

1. Check the local Server session. The address is locked once setup starts.
2. For `needs-login`, enter your existing Beeper email, request a code and enter
   the email code inside the app. New-account registration is refused.
3. For `needs-verification` or `needs-secrets`, request verification and approve
   the incoming prompt in an already-verified Beeper client (Desktop is allowed;
   a phone is not required). Refresh to see the transaction progress. Compare
   emoji/numbers on both sides before choosing "They match on both devices".
   An existing recovery key can instead be entered in the masked in-app field.
   No key is generated or reset here.
4. For `initializing` or `needs-first-sync`, refresh later. No background rapid
   polling is performed. `ready` is not proof that every network is connected.
5. Choose "Connect Solmu" only when Server is ready.

The bearer credential is stored by loopback origin in the platform OS keyring.
If saving fails, the session remains in memory with an explicit warning and a
retry-save control; there is no plaintext fallback. Subsequent launches at the
same endpoint can load that entry. Email codes/recovery keys are never written
to settings or logs. Request response bodies are not included in errors.
Redirects and non-loopback sign-in addresses are refused; local requests bypass
system proxies. A 401/403 is authentication rejection, not "Server unreachable".
A stale token never falls back to another CLI target or Desktop credential.

The current official CLI can lose target authentication when its readiness check
resolves a supplied base URL as `custom` and falls back to the default target.
The in-app flow does not use the CLI or its fallback. For CLI-only setup of the
existing target, a process-scoped workaround is:

```sh
env -u BEEPER_ACCESS_TOKEN BEEPER_TARGET=server beeper setup -t server
```

## Evidence and limitations

HTTP protocol checked against the official CLI's locked SDK commit b9c1714 and
current SDK. The SDK renamed `app.login` to `app.setup`; HTTP paths are the same:

- https://raw.githubusercontent.com/beeper/desktop-api-js/b9c1714/src/resources/app/login/login.ts
- https://raw.githubusercontent.com/beeper/desktop-api-js/next/src/resources/app/setup/setup.ts
- https://raw.githubusercontent.com/beeper/desktop-api-js/next/src/resources/app/setup/verifications/verifications.ts
- https://help.beeper.com/how-to-verify-a-beeper-app

`cargo test --manifest-path contrib/signin-check/Cargo.toml` compiles the actual
sign-in module with real UI, HTTP and keyring libraries and runs synthetic
loopback/GUI-state tests. The current adapter and launcher tests remain separate.
Visual previews exercise the real sign-in window without any user account.
Full-app compilation, real Server interop, native Keychain/restart, native Mac
launch and versioned DMG acceptance must still pass. A focused source-module
pass is not a native release or end-to-end acceptance claim.

## Read-only endpoint inventory checks

`contrib/beeper-tools/check_endpoint.py` checks the existing API inventory
without sending messages or changing read state. Its summary contains counts
and allowlisted status labels, never message bodies or account identifiers.
Malformed account/chat/history responses fail the check instead of appearing
as an empty healthy inventory. A successful inventory check does not establish
that every bridge is connected or that sending works. Use expected account and
chat counts from a previous local check to detect missing inventory.
