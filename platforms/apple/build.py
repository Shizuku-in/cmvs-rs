#!/usr/bin/env python3
"""Build the macOS player, or a standalone iOS device/simulator application."""
import argparse
import os
from pathlib import Path
import plistlib
import shutil
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("platform", choices=["macos", "ios", "ios-simulator"])
parser.add_argument("--font", type=Path, help="User-provided OpenType font bundled with the iOS app")
args = parser.parse_args()
root = Path(__file__).resolve().parents[2]
env = os.environ.copy()
env.setdefault("RUSTUP_TOOLCHAIN", "stable")
env.setdefault("CARGO_BUILD_JOBS", "4")
cargo = shutil.which("cargo") or str(Path.home()/".cargo/bin/cargo")
def run(command):
    subprocess.run(command, cwd=root, env=env, check=True)
if args.platform == "macos":
    run([cargo, "build", "--locked", "--release", "--features", "desktop,audio-output", "--bin", "cmvs-desktop"])
    raise SystemExit(0)
if not args.font or not args.font.is_file():
    parser.error("iOS requires --font with a readable OpenType font")
simulator = args.platform == "ios-simulator"
triple = "aarch64-apple-ios-sim" if simulator else "aarch64-apple-ios"
sdk = "iphonesimulator" if simulator else "iphoneos"
env["IPHONEOS_DEPLOYMENT_TARGET"] = "15.0"
run([cargo, "build", "--locked", "--release", "-p", "cmvs-host", "--target", triple])
app = root/"target/apple"/triple/"Cmvs.app"
app.mkdir(parents=True, exist_ok=True)
sdk_path = subprocess.check_output(["xcrun", "--sdk", sdk, "--show-sdk-path"], text=True).strip()
run(["xcrun", "--sdk", sdk, "clang++", "-std=c++17", "-fobjc-arc", "-O2", "-g",
     "-target", "arm64-apple-ios15.0"+("-simulator" if simulator else ""), "-isysroot", sdk_path,
     "-I", str(root/"crates/cmvs-host"), str(root/"platforms/apple/ios/main.mm"),
     str(root/"target"/triple/"release/libcmvs_host.a"), "-o", str(app/"Cmvs"),
     "-framework", "UIKit", "-framework", "UniformTypeIdentifiers", "-framework", "AVFAudio",
     "-framework", "AudioToolbox", "-framework", "CoreAudio", "-framework", "CoreFoundation",
     "-framework", "Foundation", "-framework", "CoreGraphics", "-framework", "QuartzCore",
     "-framework", "Security", "-framework", "SystemConfiguration", "-lobjc", "-lc++"])
shutil.copy2(args.font, app/"font.otf")
with (app/"Info.plist").open("wb") as file:
    plistlib.dump({"CFBundleExecutable":"Cmvs", "CFBundleIdentifier":"org.cmvs.player",
                  "CFBundleName":"CMVS", "CFBundleDisplayName":"CMVS", "CFBundlePackageType":"APPL",
                  "CFBundleVersion":"1", "CFBundleShortVersionString":"0.1", "MinimumOSVersion":"15.0",
                  "CFBundleSupportedPlatforms":["iPhoneSimulator" if simulator else "iPhoneOS"],
                  "UIDeviceFamily":[1,2], "UILaunchScreen":{},
                  "UISupportedInterfaceOrientations":["UIInterfaceOrientationLandscapeLeft","UIInterfaceOrientationLandscapeRight"],
                  "UIFileSharingEnabled":True, "LSSupportsOpeningDocumentsInPlace":True}, file)
if simulator:
    run(["codesign", "--force", "--sign", "-", str(app)])
print(app)
if not simulator:
    print("Device app is unsigned. Sign with your provisioning profile before installing.")
