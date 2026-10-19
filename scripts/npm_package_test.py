"""The npm package's bin is the native binary itself: no node shim, no
install script. A fresh user's install must run the binary directly."""
import json
import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
NPM = shutil.which("npm")


@unittest.skipUnless(NPM, "npm is not installed")
class NpmPackageTests(unittest.TestCase):
    def test_global_install_links_the_native_binary(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            pkg = tmp / "pkg"
            shutil.copytree(ROOT / "npm", pkg, ignore=shutil.ignore_patterns("bin", "runner"))
            (pkg / "bin").mkdir()
            native = pkg / "bin" / "agent-mobile"
            native.write_text("#!/bin/sh\necho agent-mobile stand-in\n")
            native.chmod(0o755)
            (pkg / "runner").mkdir()
            (pkg / "runner" / "AgentMobileDriver.xctestrun").write_text("<plist/>")
            env = {**os.environ, "HOME": str(tmp), "npm_config_prefix": str(tmp / "prefix"),
                   "SFW_SHIM_DISABLE": "1"}
            subprocess.run([NPM, "pack", "--pack-destination", str(tmp)], cwd=pkg, env=env,
                           check=True, capture_output=True)
            tarball = next(tmp.glob("agent-mobile-*.tgz"))
            # --force: the package is darwin-only and CI also runs on Linux.
            subprocess.run([NPM, "install", "-g", "--force", str(tarball)], cwd=tmp, env=env,
                           check=True, capture_output=True)
            entry = tmp / "prefix" / "bin" / "agent-mobile"
            installed = tmp / "prefix" / "lib" / "node_modules" / "agent-mobile" / "bin" / "agent-mobile"
            self.assertEqual(os.path.realpath(entry), os.path.realpath(installed))
            run = subprocess.run([str(entry)], capture_output=True, text=True, check=True)
            self.assertEqual(run.stdout.strip(), "agent-mobile stand-in")

    def test_manifest_has_no_install_scripts(self):
        manifest = json.loads((ROOT / "npm" / "package.json").read_text())
        hooks = {"preinstall", "install", "postinstall"} & set(manifest.get("scripts", {}))
        self.assertFalse(hooks, "pnpm and bun block install scripts and warn every fresh user")


if __name__ == "__main__":
    unittest.main()
