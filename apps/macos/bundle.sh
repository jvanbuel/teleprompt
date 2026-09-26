#!/bin/sh
# Builds Teleprompt.app here: a release build of the package, with the
# Info.plist macOS needs to ask for the microphone, signed ad hoc.
set -eu
cd "$(dirname "$0")"
swift build -c release --product Teleprompt
bin=$(swift build -c release --show-bin-path)
app=Teleprompt.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp "$bin/Teleprompt" "$app/Contents/MacOS/Teleprompt"
mkdir -p "$app/Contents/Resources/Fonts"
cp ../fonts/*.ttf "$app/Contents/Resources/Fonts/"
iconutil -c icns AppIcon.iconset -o "$app/Contents/Resources/AppIcon.icns"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>Teleprompt</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>ATSApplicationFontsPath</key><string>Fonts</string>
  <key>CFBundleIdentifier</key><string>io.github.jvanbuel.teleprompt</string>
  <key>CFBundleName</key><string>Teleprompt</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key>
  <string>Teleprompt listens to you read, to follow you through the script and record your takes.</string>
</dict>
</plist>
PLIST
codesign --force --sign - "$app"
echo "built $PWD/$app"
