#!/bin/sh
set -eu
cd "$(dirname "$0")"
repo_root=$(cd ../../../.. && pwd)
# Resolve once; launchd's fixed PATH needs Nix, not a mutable rustup installation.
toolchain=$(nix build --no-link --print-out-paths "$repo_root#ios-rust-toolchain")
export PATH="$toolchain/bin:$PATH"
export RUSTC="$toolchain/bin/rustc"
# Both Rust linking and cc-rs C/assembly compilation require Apple's compiler.
export CARGO_TARGET_AARCH64_APPLE_IOS_LINKER=/usr/bin/clang
export CARGO_TARGET_AARCH64_APPLE_IOS_SIM_LINKER=/usr/bin/clang
export CC_aarch64_apple_ios=/usr/bin/clang
export CC_aarch64_apple_ios_sim=/usr/bin/clang
# Match Expo's iOS minimum, including C/assembly dependencies built by cc-rs.
export IPHONEOS_DEPLOYMENT_TARGET=16.4
mkdir -p build/headers ios/build
cp ios/FractalFabricBridge.h build/headers/
cat > build/headers/module.modulemap <<'MAP'
module FractalFabricRust {
  header "FractalFabricBridge.h"
  export *
}
MAP
for target in aarch64-apple-ios aarch64-apple-ios-sim; do
  case "$target" in
    aarch64-apple-ios)
      fabric_minimum_flag=-miphoneos-version-min=16.4
      fabric_sdk=iphoneos
      ;;
    aarch64-apple-ios-sim)
      fabric_minimum_flag=-mios-simulator-version-min=16.4
      fabric_sdk=iphonesimulator
      ;;
  esac
  SDKROOT=$(/usr/bin/xcrun --sdk "$fabric_sdk" --show-sdk-path)
  export SDKROOT
  # blake3 tracks CFLAGS but not Apple's deployment environment when reusing C objects.
  CFLAGS="${CFLAGS:-} $fabric_minimum_flag" "$toolchain/bin/cargo" build --manifest-path rust/Cargo.toml --release --locked --target "$target"
done
# Other native projects share this host. Never overlap their xcodebuild process.
while pgrep -x xcodebuild >/dev/null; do sleep 10; done
rm -rf ios/build/FractalFabricRust.xcframework
xcodebuild -create-xcframework \
  -library rust/target/aarch64-apple-ios/release/libfractal_fabric_bridge.a -headers build/headers \
  -library rust/target/aarch64-apple-ios-sim/release/libfractal_fabric_bridge.a -headers build/headers \
  -output ios/build/FractalFabricRust.xcframework > build/xcframework.log 2>&1
