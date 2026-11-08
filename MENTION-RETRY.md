# Mention retry and composer transitions

| Input/state | Result |
| --- | --- |
| Failed text, empty composer | Restore editable text and exact recipient selections; never send |
| Failed text, existing draft or active recording | Keep existing draft/selections; never send |
| Repeated same recipient | Reuse its editable label, encode every occurrence, deduplicate recipient IDs |
| Different recipients with same name | Give each a unique editable label; longest label matches first |
| Original names already contain a generated suffix | Reserve existing names; choose unused suffix |
| Legacy metadata gives different recipients identical labels | Keep ambiguous display text literal; do not guess a recipient |
| Invalid/no-longer-present selected recipient | Do not encode that selection |
| User edits away a selected mention | Do not add its ID to outgoing mention metadata |
| One display name equals another recipient's numeric wire ID | Match original draft spans once; never re-interpret generated wire tokens |
| Markup/repeated mentions/non-ASCII text | Preserve original markup/text while replacing only recognized tokens |
| Linked retry changes to its exact source chat | Forward both text and mention selections, with the same draft guards |

Reaction clocks have their own measured footer region. Reaction pills start
below its bottom. Retry is a named accessible button. Footer label measurement
uses the same translated text as rendering. Unknown status remains inspectable,
without adding a standalone transcript clock gutter.

Verification: composer/retry round-trip tests, normal A/B/A same-name selection,
legacy ambiguous label and generated-token collision regressions, reserved-name
and repeated token markup tests, theme reaction-spacing/accessibility tests,
and the full UI Tab-cycle test in CI. No screenshots are committed here.

The final audit's six focused native app tests pass, using unchanged production
code and omitting unrelated test modules. Strict all-feature/all-target clippy
passes. Prior reaction/theme/accessibility and full Tab-cycle checks passed in
CI at c4de392. The one-pass encoding change still needs full current-head CI.
