#!/usr/bin/env python3
"""Repository task entry point for application-owned group provisioning."""

from pathlib import Path
import runpy

if __name__ == "__main__":
    runpy.run_path(
        str(Path(__file__).resolve().parents[2] / "applications/tools/group-provisioning/provision.py"),
        run_name="__main__",
    )
