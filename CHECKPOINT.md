# Fast-beeper coordination and development checkpoint

## Automatic chat categories - November 10, 2026

Branch `feat/chat-auto-categories`, based on main 4a55ca9, version 0.24.13. First slice of option 4: Filters > Automatic category (People, Quiet people, Notifications) from plain rules in `src/categories.rs`, plus Move to... / Sort automatically on a one-to-one chat's menu, stored in `beeper.category_overrides`. Tests in `src/categories.rs` and `tests/chat_scope.rs`. Not compiled locally (no libpipewire headers here); CI is the first compile. No demo captures yet. Details and gaps: docs/chat-categories.md. No merge by the author.
