#!/usr/bin/env bash
# App-specific checks retained alongside shared packaging lint.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

bash packaging/server/install-test.sh
if grep -rniE 'photoshop|psd|photocraft|vectorcraft|illustrator|adobe|figma' packaging/ .github/ \
    --exclude=packaging-lint.yml --exclude=check-app-specific.sh; then
  echo "::error::package metadata mentions another product (see above)" >&2
  exit 1
fi

# The Nightly's app-side synthetic suites: the icon containers and provenance, the macOS metadata
# and signing scripts against stand-in Apple tools, the caller contracts, and the Arch tarball. No
# app build, no signing material and no network.
python3 -I packaging/nightly-tests.py
bash packaging/linux/nightly-launch-test.sh
bash packaging/linux/nightly-arch-tar-test.sh
# Every Nightly shell script must parse: a quoting slip breaks bash -n, and nothing else would
# catch it until a runner started the script for real.
for script in packaging/nightly-common.sh packaging/linux/nightly.sh packaging/linux/nightly-launch.sh \
  packaging/linux/nightly-package-launch.sh \
  packaging/linux/nightly-launch-test.sh \
  packaging/linux/nightly-arch-tar.sh packaging/macos/nightly-lib.sh packaging/macos/nightly.sh \
  packaging/macos/nightly-sign.sh packaging/macos/nightly-verify.sh; do
  bash -n "$script"
done

python3 - <<'PYEOF'
import yaml
manifests = []
for name in ("ai.storyteller.figcraft.yml", "ai.storyteller.figcraft.bundle.yml"):
    with open("packaging/linux/flatpak/" + name) as source:
        manifest = yaml.safe_load(source)
    assert manifest["id"] == "ai.storyteller.figcraft" and manifest["command"] == "figcraft", name
    manifests.append(manifest)
for key in ("runtime", "runtime-version", "sdk", "finish-args"):
    assert manifests[0][key] == manifests[1][key], f"Flatpak manifests disagree on {key}"
print("FigCraft package identity ok")
PYEOF
