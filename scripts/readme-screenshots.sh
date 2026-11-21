#!/bin/bash
set -e
cd "$(dirname "$0")/../.."
rm -rf build/screenshots/r.xcresult build/screenshots/out; mkdir -p build/screenshots/out
TEST_RUNNER_GEWU_SCREENSHOT_DATA=$PWD/build/screenshots/yellow_tripdata_2024-01.parquet xcodebuild -project Gewu.xcodeproj -scheme GewuScreenshots -destination 'platform=macOS' -derivedDataPath build/xcode -resultBundlePath build/screenshots/r.xcresult test 2>&1 | grep -E " error|Test Case|BUILD" | head -20
xcrun xcresulttool export attachments --path build/screenshots/r.xcresult --output-path build/screenshots/out
ls -la build/screenshots/out
