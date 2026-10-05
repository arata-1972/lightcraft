#!/usr/bin/env python3
"""Package the locally built arm64 app with an independent name and bundle identifier."""
import argparse
import pathlib
import plistlib
import shutil
import subprocess

root = pathlib.Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument("--output", type=pathlib.Path, required=True)
options = parser.parse_args()
app = options.output.resolve()
if app.exists():
    raise SystemExit(f"Output already exists: {app}")
for name in ("lightcraft", "lightcraft-cli"):
    if not (root / "target/release" / name).is_file():
        raise SystemExit(f"Build {name} first with cargo build --release")

macos = app / "Contents/MacOS"
resources = app / "Contents/Resources"
macos.mkdir(parents=True)
resources.mkdir(parents=True)
template = (root / "packaging/macos/Info.plist.in").read_text()
revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
template = template.replace("@VERSION@", "0.2.0-ja.1").replace("@SHORT_VERSION@", "0.2.0").replace("@BUILD_SHA@", revision)
info = plistlib.loads(template.encode())
info.update({
    "CFBundleDevelopmentRegion": "ja",
    "CFBundleDisplayName": "LightCraft 日本語",
    "CFBundleName": "LightCraft 日本語",
    "CFBundleIdentifier": "local.taigi.lightcraft.japanese",
    "CFBundleLocalizations": ["ja", "en"],
    "NSHumanReadableCopyright": "Based on LightCraft by its contributors. Japanese localization added locally. MIT OR Apache-2.0.",
})
# This local build is an editor, not a competing system document-type owner.
info.pop("UTExportedTypeDeclarations", None)
info.pop("UTImportedTypeDeclarations", None)
(app / "Contents/Info.plist").write_bytes(plistlib.dumps(info))
shutil.copy2(root / "target/release/lightcraft", macos / "LightCraft")
shutil.copy2(root / "target/release/lightcraft-cli", macos / "lightcraft-cli")
shutil.copy2(root / "assets/app-icon/lightcraft.icns", resources / "LightCraft.icns")
licenses = resources / "Licenses"
licenses.mkdir()
for source in ("LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "assets/fonts/OFL-Inter.txt", "assets/fonts/OFL-BIZUDGothic.txt", "assets/fonts/OFL-BIZUDMincho.txt", "assets/app-icon/LICENSE.txt"):
    shutil.copy2(root / source, licenses / ("Icon-LICENSE.txt" if source == "assets/app-icon/LICENSE.txt" else pathlib.Path(source).name))
shutil.copy2(root / "docs/localization-ja.md", resources / "日本語版について.md")
subprocess.run(["codesign", "--force", "--sign", "-", "--timestamp=none", str(macos / "lightcraft-cli")], check=True)
subprocess.run(["codesign", "--force", "--sign", "-", "--timestamp=none", str(app)], check=True)
subprocess.run(["codesign", "--verify", "--deep", "--strict", str(app)], check=True)
print(app)
