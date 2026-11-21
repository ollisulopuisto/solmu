# Solmu scoped API (experimental, local only)

`solmu-reader` lets other apps on the same machine read messages and, with a
separate exact-destination send grant, send text with their own revocable credentials. It runs beside the Beeper
Server, like `solmu-scheduler`. This is code and docs only: nothing here
is installed, deployed, or granted to any app.

## What exists and what it adds

- Beeper's own Server/Desktop API needs a token on every call, but a token
  is all-or-nothing and can send messages. Handing it to another app is not
  least privilege. It is left alone and never exposed.
- The Send Later daemon has one shared control token and can queue sends.
  It is not reused here.
- `solmu-reader` holds the Server token itself. Existing read tokens remain
  read-only. Text send requires `messages:send` and explicit account/chat pairs.
  Edit, delete, read-marker, reaction and focus endpoints remain unroutable.

## Endpoints

| Scope | Endpoint |
|---|---|
| `accounts:read` | `GET /v1/accounts` |
| `chats:read` | `GET /v1/chats` |
| `messages:read` | `GET /v1/chats/{id}/messages` |
| `search:read` | `GET /v1/messages/search?query=` |
| `people:read` | `GET /v1/people`, `GET /v1/people/{id}/messages?limit=` |

## Linked people (Solmu's own view)

Set `SOLMU_SETTINGS_FILE` to the app's `settings.json` (read-only; only
`beeper.people` is read, nothing else in the file is parsed into a
response). `/v1/people` lists linked persons: a content-derived `id`, name,
primary destination and members, each marked `readable`. `/v1/people/{id}/messages`
merges the newest messages across the person's readable members (limit
capped at 100, no paging), adds `chatID`/`accountID` to each item, and lists
every member it could not read under `unavailable` with `partial: true`.
- A member's `origin` is the API base URL the app read that chat from
  (BeeperConfig base URL, e.g. `http://127.0.0.1:23374`). The reader serves
  members whose origin is in `SOLMU_READER_ORIGINS` (comma list; default
  `BEEPER_BASE_URL`). Compare with what the app records: a tunnel on the
  laptop and the Server's own URL differ, so set the list to the app's
  value. Other origins are listed `readable: false`, left out of merged
  timelines and named in `unavailable`.
- Each readable member's chat is checked against the Server (it must exist
  and belong to the recorded account) before its messages are read.
- A restricted client sees a person only if every member is inside its
  account allowlist, so a cross-account link is never revealed.
- Without `SOLMU_SETTINGS_FILE`, or if the file is unreadable or over
  4 MiB, people endpoints return 502.

Query keys are limited to `cursor`, `direction` (before/after), `limit`
(capped at 100) and, for search, `query`. Anything else is refused with 400.
Responses come from the Server unchanged for unrestricted clients.

### Validating the origin match (non-secret)

`SOLMU_SETTINGS_FILE=<settings.json> solmu-reader people-check` lists
each origin recorded in the app's people links, how many members use it,
and whether this reader serves it; it exits 1 if any is unserved. The same
lines are logged at startup. It prints only API base URLs and counts: no
names, chat ids or message content, and it never contacts the Server.

## v1 coverage (supported, explicit)

One configured Beeper origin, read through that Server's API, plus the
linked-person view over members recorded under that origin. Not claimed:
multiple origins, direct protocol login, or live delivery verification.

## Authentication and authorization

- One named token per app: `solmu-reader token add NAME --scopes a,b
  [--accounts x,y] [--days N]`. The token prints once on the terminal.
  Only its SHA-256 is stored, in `~/.config/zapfast/reader-clients.json`
  (mode 0600 from creation; a looser file is refused on load).
- `token list` shows names, scopes, accounts and expiry. `token revoke NAME`
  removes the entry. The file is re-read per request, so revocation is
  immediate, no restart.
- `--accounts` restricts a client to those Beeper account ids. Listings and
  search are filtered by account; everything except the filtered `items`
  (counts, cursors, hasMore, lookup maps) is dropped, so restricted clients
  get one page and no paging; a chat's
  messages are served only if the chat belongs to an allowed account, and
  otherwise look like 404. An unexpected response shape fails closed (502).
  Chat-level allowlists are not supported: a registry entry with unknown
  fields is rejected, and a bad registry refuses all requests.
- Constant-time hash comparison, 120 requests per minute per token,
  loopback bind only (non-loopback addresses refuse to start), requests
  with Origin or Referer headers refused, 5 s read timeout, 16 KiB request
  cap, `cache-control: no-store`, upstream error bodies never forwarded.
- Log lines hold client name, endpoint family and status. Never query
  strings, chat ids or message content. The token CLI never reads the
  upstream token.

## Not covered (explicitly)

- Chats served by another API origin are not served. Current app code creates
  only the Beeper backend. Inherited native-WhatsApp-mode/broker design notes
  describe future or historical paths, not a second live backend in this fork.
- The scheduled queue is not exposed through this API.
- No push or websocket: clients poll.
- No remote access. Reaching it from another machine (SSH tunnel, VPN) is a
  separate, later decision.
- Connections have separate bounded worker slots. Slow clients occupy their
  own slot until the total header/body deadlines expire.

## Unverified

- Whether the Beeper Server marks a chat read when its messages are fetched
  with GET. The Server docs do not say. This API never calls the read or
  unread endpoints, but the upstream behavior needs a live check before the
  API is described as side-effect free.
- Live response shapes (`items`, `accountID` on chats and search results)
  are taken from the app's own parsing. Restricted clients depend on them
  and fail closed if they differ.
- Behavior with a real Server and real client app: untested.

## Run (manual)

    export BEEPER_ACCESS_TOKEN_FILE=~/.config/zapfast/server-token
    solmu-reader token add dashboard --scopes chats:read,messages:read
    solmu-reader                       # serves 127.0.0.1:23377
    curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:23377/v1/chats

## Open security work before this can be called ready

- HTTP-level tests (tests/read_api_http.rs) cover auth, Origin/Referer,
  traversal, probe, redirect, error bodies, revocation, rate limit,
  connection cap and registry permissions against a mock Server. They have
  not been run against a real Server.
- Each connection has its own thread (cap 32) and its own upstream handle,
  so a slow Server call delays only its own connection (up to 30 s per
  call, several calls for a merged timeline).
- `token add/revoke/list` take an exclusive lock file; a lock is recovered
  only when the process that wrote it (pid recorded in the file) is no longer
  alive; age alone never breaks it. Pid reuse after a crash can keep a stale
  lock until it is removed by hand.
- Live check of Server side effects (see Unverified) and of response shapes.
- Other-origin chats and direct protocol login are not built.
- Linked-person endpoints are tested only against synthetic settings and a
  mock Server.

## Scoped text sends, CLI and MCP

This implementation serves one configured Beeper Server/Desktop origin, matching
the app's current backend, including WhatsApp connected through Beeper. It does
not add direct WhatsApp protocol login or remove the existing Server dependency.
No service, token or send is created merely by building these binaries.

To grant sending, create a private JSON file containing only the exact intended
account/chat pairs, for example synthetic `[{"account":"a","chat":"c"}]`, then run:

```sh
solmu-reader token add MY_APP --scopes messages:send --accounts a --days 7 --send-destinations-file /path/to/destinations.json
```

The token appears once. Store it in a private file (0600 on Unix), not a shell
argument, source control or a shared log. Account restrictions and destination
pairs are both enforced. Omission of the destination file never grants sending.
Existing `token revoke NAME` revokes both reads and sends. After revocation,
expiry or changed grants, in-flight sends revalidate before committing upstream;
a send already committed cannot be recalled by revocation.

`GET /v1/capabilities` requires a valid bearer and returns that client's scopes,
account restrictions, exact send destinations and expiry, never its token.
`POST /v1/chats/{percent-encoded-id}/messages` requires JSON with only
`account_id`, `text`, `request_id`. Text is bounded to 16 KiB and JSON to 20 KiB.
It sends one text, without attachments, quotes, mentions or implicit recipient
selection. Write requests are separately limited to 10/minute/token. Browser
Origin/Referer, duplicate headers, transfer encoding and unbounded bodies are
refused; the listener remains loopback-only. No MCP HTTP endpoint is added.

```sh
solmu-control send --token-file /private/client-token --account a --chat c --text-file /private/message.txt --request-id stable-unique-id
solmu-mcp --token-file /private/client-token
```

Both clients default to `http://127.0.0.1:23377`; optional `--url` accepts only
literal loopback HTTP origins. Redirects and proxy-environment routing are disabled.
MCP uses bounded newline-delimited JSON-RPC over stdio, supports modern 2026-07-28
per-request metadata/server discovery and explicit 2025-11-25 initialization for
older clients. Its tool list follows the current token's scopes. Actual tool
calls always pass through the same server authorization and write journal.
Tools are accounts, chats, messages, search, people and send_text. Callers need
human authorization for their own actions; tool annotations do not grant consent.
No secret or diagnostic text is emitted to MCP stdout.

Write receipt metadata and claim/outcome records are saved beside the client
registry in `reader-clients.writes.json` (0600 on Unix). The journal contains
hashed client identities, timestamps, request IDs, payload digests and outcomes,
not plaintext message bodies or credentials. It refuses writes if persistence
fails or its bounded history fills; archive it deliberately, never delete it to
force a retry. Reusing an ID with different content or destination is refused.
Claimed/unknown writes are never automatically resent, including after a crash.
A successful response means accepted, not delivered. Never use a new request ID
to retry an uncertain send. Read-state side effects of upstream GET endpoints
have not been live-verified. No account/network terms exemption is implied.

Protocol references:
- https://modelcontextprotocol.io/specification/2026-07-28/basic/lifecycle
- https://modelcontextprotocol.io/specification/2026-07-28/server/tools
- https://modelcontextprotocol.io/specification/2026-07-28/server/discover
