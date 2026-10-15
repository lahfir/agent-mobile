#!/usr/bin/env python3
"""Benchmark entry point: delegates to scripts.benchkit (see that package)."""
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))

from scripts.benchkit.cli import main

if __name__ == "__main__":
    main()
