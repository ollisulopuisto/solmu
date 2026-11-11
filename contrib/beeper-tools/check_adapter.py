#!/usr/bin/env python3
"""Run portable checks against current sources, retaining explicit leaf stubs."""
from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import tempfile

# These are real modules, not harness-specific leaf stubs. Always stage them
# from src/ so a copied harness snapshot cannot hide an active-source regression.
LIVE_MODULES = (
    'account.rs', 'categories.rs', 'backend.rs', 'backend/beeper.rs', 'backend/beeper/gallery.rs', 'backend/local_ai.rs', 'backend/local_files.rs', 'backend/linked_media.rs',
    'composed_forward.rs', 'group_admin.rs', 'i18n.rs', 'mentions.rs', 'merged.rs', 'model.rs', 'suggest.rs', 'pending_media.rs',
    'new_group.rs', 'paths.rs', 'run_widths.rs', 'safety.rs', 'saved.rs', 'scheduler.rs',
    'text_replace.rs',
    'scheduler_client.rs',
)

def stage(root: Path, destination: Path) -> None:
    shutil.copytree(root / 'harness', destination,
                    ignore=shutil.ignore_patterns('target', '__pycache__'))
    for relative in LIVE_MODULES:
        source = root / 'src' / relative
        if not source.is_file():
            raise FileNotFoundError(f'Missing live adapter source: {source}')
        (destination / 'src' / relative).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination / 'src' / relative)


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    work = root / 'target' / 'adapter-check'
    work.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='source-', dir=work) as tmp:
        destination = Path(tmp) / 'harness'
        stage(root, destination)
        env = os.environ.copy()
        env.setdefault('CARGO_TARGET_DIR', str(root / 'harness' / 'target'))
        return subprocess.call(
            ['cargo', 'test', '--locked', '--manifest-path',
             str(destination / 'Cargo.toml')], cwd=root, env=env)


if __name__ == '__main__':
    raise SystemExit(main())
