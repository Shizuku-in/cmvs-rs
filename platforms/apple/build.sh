#!/usr/bin/env sh
set -eu

usage() {
    echo 'Usage: build.sh xcframework [macos] [ios] [ios-simulator]'
    echo '       build.sh {macos|ios|ios-simulator} [--configuration Debug|Release] [--reuse-framework]'
}

invalid() {
    echo "$1" >&2
    usage >&2
    exit 2
}

platform=
slices=
configuration=Release
reuse=false
while [ "$#" -gt 0 ]; do
    case "$1" in
        -h|--help) usage; exit 0 ;;
        --reuse-framework) reuse=true ;;
        --configuration)
            [ "$#" -ge 2 ] || invalid 'Missing configuration'
            shift
            configuration=$1 ;;
        --configuration=*) configuration=${1#*=} ;;
        xcframework)
            [ -z "$platform" ] || invalid 'Specify one platform'
            platform=$1 ;;
        macos|ios|ios-simulator)
            case "$platform" in
                '') platform=$1 ;;
                xcframework) slices="$slices $1" ;;
                *) invalid 'Specify one platform' ;;
            esac ;;
        *) invalid "Unknown argument: $1" ;;
    esac
    shift
done
[ -n "$platform" ] || invalid 'Missing platform'
case "$configuration" in
    Debug|Release) ;;
    *) invalid "Unknown configuration: $configuration" ;;
esac

CDPATH= cd -P "$(dirname "$0")"
apple_dir=$PWD
repo_dir=$(cd ../.. && pwd -P)
target_dir=$repo_dir/target
out=$target_dir/apple
framework=$out/CmvsApple.xcframework

build_framework() (
    cd "$repo_dir"
    export MACOSX_DEPLOYMENT_TARGET=14.0 IPHONEOS_DEPLOYMENT_TARGET=17.0 CARGO_TARGET_DIR="$target_dir"
    mkdir -p "$out"
    stage=$(mktemp -d "$out/xcframework.XXXXXX")
    trap 'rm -rf "$stage"' EXIT
    trap 'exit 1' HUP INT TERM
    archive=release/libcmvs_apple.a
    set -- -create-xcframework
    for slice in ${slices:-macos ios ios-simulator}; do
        case "$slice" in
            macos) targets='aarch64-apple-darwin x86_64-apple-darwin' ;;
            ios) targets=aarch64-apple-ios ;;
            ios-simulator) targets=aarch64-apple-ios-sim ;;
        esac
        for target in $targets; do
            cargo build --locked --release -p cmvs-apple --target "$target"
        done
        if [ "$slice" = macos ]; then
            library=$stage/libcmvs_apple.a
            lipo -create "$target_dir/aarch64-apple-darwin/$archive" "$target_dir/x86_64-apple-darwin/$archive" -output "$library"
        else
            library=$target_dir/$targets/$archive
        fi
        set -- "$@" -library "$library" -headers "$apple_dir/native/include"
    done
    xcodebuild "$@" -output "$stage/CmvsApple.xcframework"
    rm -rf "$framework"
    mv "$stage/CmvsApple.xcframework" "$framework"
)

if [ "$platform" = xcframework ] || [ "$reuse" = false ]; then
    build_framework
fi
[ "$platform" != xcframework ] || exit 0

scheme=CmvsIOS
archs=arm64
sign=YES
case "$platform" in
    macos) sdk=macosx; scheme=CmvsMac; archs='arm64 x86_64'; set -- aarch64-apple-darwin x86_64-apple-darwin ;;
    ios) sdk=iphoneos; sign=NO; set -- aarch64-apple-ios ;;
    ios-simulator) sdk=iphonesimulator; set -- aarch64-apple-ios-sim ;;
esac

sh notices.sh "$out/DEPENDENCY-LICENSES.txt" "$@"
set --
if [ -n "${CMVS_VERSION:-}" ]; then
    set -- "MARKETING_VERSION=$CMVS_VERSION" "CURRENT_PROJECT_VERSION=${CMVS_BUILD_NUMBER:-1}"
fi
xcodegen generate
rm -rf "$out/$platform/CMVS.app"
xcodebuild -project Cmvs.xcodeproj -scheme "$scheme" -configuration "$configuration" \
    -sdk "$sdk" -derivedDataPath "$out/DerivedData" ONLY_ACTIVE_ARCH=NO "ARCHS=$archs" \
    "CODE_SIGNING_ALLOWED=$sign" "CONFIGURATION_BUILD_DIR=$out/$platform" "$@" build
echo "$out/$platform/CMVS.app"
