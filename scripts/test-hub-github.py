#!/usr/bin/env python3
"""Exercise private-release discovery without accessing the user's account."""

import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import hub_github as github


class GitHubTests(unittest.TestCase):
    def test_repository_and_asset_arguments(self):
        for name in ("--repo/x", "a/b/c", "https://github.com/a/b", "a/b c"):
            with self.assertRaises(ValueError):
                github.repository(name)
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "editor.zip"
            with patch.object(github, "cli", return_value="gh"), patch.object(github, "run") as run:
                github.asset("Blockworked/Blockloom", "0.1.0", "editor.zip", path)
                self.assertEqual(run.call_args.args[0], ["gh", "release", "download", "0.1.0", "--repo",
                    "Blockworked/Blockloom", "--pattern", "editor.zip", "--output", str(path)])
                with self.assertRaises(ValueError):
                    github.asset("a/b", "--clobber", "editor.zip", path)

    def test_missing_cli_explains_authentication(self):
        with patch.object(github.shutil, "which", return_value=None):
            with self.assertRaisesRegex(ValueError, "gh auth login"):
                github.cli()

    def test_catalog_uses_authenticated_assets(self):
        entry = {"version": "0.1.0", "target": "fixture", "asset": "editor.zip",
                 "url": "https://github.com/Blockworked/Blockloom/releases/download/0.1.0/editor.zip",
                 "size": 12, "unpacked_size": 20, "sha256": "0" * 64, "format": "zip"}
        release = {"tag_name": "0.1.0", "assets": [{"name": github.CATALOG_ASSET, "size": 500}, {"name": "editor.zip", "size": 12}]}

        def asset(repo, tag, name, path):
            path.write_text(json.dumps({"schema": 1, "releases": [entry]}))

        with patch.object(github, "capture", return_value=[release]), patch.object(github, "asset", side_effect=asset):
            entries = github.catalog(github.DEFAULT_REPO, "fixture")
            self.assertEqual(entries[0]["github"], {"repo": github.DEFAULT_REPO, "tag": "0.1.0", "asset": "editor.zip"})
        with patch.object(github, "capture", return_value=[release, release]), patch.object(github, "asset", side_effect=asset):
            with self.assertRaisesRegex(ValueError, "duplicate"):
                github.catalog(github.DEFAULT_REPO, "fixture")
        release["assets"][-1]["size"] = 13
        with patch.object(github, "capture", return_value=[release]), patch.object(github, "asset", side_effect=asset):
            with self.assertRaisesRegex(ValueError, "different size"):
                github.catalog(github.DEFAULT_REPO, "fixture")


if __name__ == "__main__":
    unittest.main()
