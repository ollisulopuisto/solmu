# Sentinel's Journal - Critical Learnings Only

## 2026-11-01 - External URL Scheme Validation at Desktop Launcher Hook
**Vulnerability:** The Linux detached link opener (`DetachedLinks`) intercepted egui's `OpenUrl` commands and forwarded them directly to `open::that_detached` without verifying the URL scheme. If untrusted schemes (e.g. `file://`, `javascript:`, or desktop protocol handlers) were routed through `OpenUrl`, they would be passed directly to the desktop launcher.
**Learning:** Even when URL filtering is applied in certain action dispatch handlers (e.g., in `App::update` for chat links), egui UI components (such as hyperlinks or plugins) can emit `OpenUrl` commands directly into `output.platform_output.commands`. Defense-in-depth requires validation at the launcher plugin boundary itself.
**Prevention:** Always validate URLs against `crate::safety::external_url` at the `DetachedLinks::output_hook` boundary prior to invoking system launchers.

## 2026-11-02 - Multi-Dot DOS Device Name Bypass in File Opener Safety
**Vulnerability:** `can_open_attachment` relied on `path.file_stem()`, which only strips the final extension. A filename with multiple dots like `CON.custom.pdf` had `file_stem() == "CON.custom"`, bypassing `is_reserved_dos_name` and allowing the file to be opened by desktop handlers on Windows, where any file whose primary base matches a DOS device name targets the reserved device.
**Learning:** Windows treats reserved device names as special when matched at the primary prefix before any dot (or after stripping extensions). Checking only the standard `Path::file_stem()` is insufficient for multi-extension files.
**Prevention:** Check both the primary stem before any period (`name.split('.').next()`) and `Path::file_stem()` against `is_reserved_dos_name`, and maintain classic DOS device names including `CLOCK$` in the reservation list.
