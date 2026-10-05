# TMP NAM app icon: handoff to Claude Code

The design is final: **option B, the gain knob**. This pack has every platform's ready-made icon files plus the vector sources and the script that made them. Your job is to wire the right files into the project. Don't redraw or restyle anything.

Design canvas (source of truth for the look): https://claude.ai/artifact/NpYod3TnkwjLWLN4ptP57w

## Prompt to paste into Claude Code

> Install the TMP NAM app icon from `TMP-NAM-Icons/` (read `TMP-NAM-Icons/HANDOFF.md` first). Detect how this project is built (Xcode, Tauri, Electron, Flutter or other), then copy the matching files from the pack into the place that build system expects, replacing any existing app icon. Don't modify, resize or re-export the PNGs, except where HANDOFF.md says a tool generates them. Then build the app and confirm the new icon is used. List every file you added or replaced.

## The design in one paragraph

A knurled amp knob seen from above, turned to about two o'clock, on a graphite tile. A blue arc running from seven o'clock to the pointer shows the level. The colours match the app's design system: the graphite gradient runs from `#3A3A40` to `#161618`, the arc goes from `#4C7DF2` to `#93B3FF` with an `#3A6AE0` glow, and the pointer is `#EEF0F5`. Every size of 32 px and under uses a simplified drawing: thicker arc and pointer, no knurling, no glow. The PNGs already switch between the two drawings, so never downscale a large PNG to make a small one.

## What's in the pack

```
TMP-NAM-Icons/
├── HANDOFF.md
├── macos/
│   ├── AppIcon.appiconset/      Xcode asset catalog set (16–512 pt, @1x/@2x) + Contents.json
│   ├── AppIcon.icns             same images as one .icns (for non-Xcode builds)
│   ├── AppIcon-1024.png         1024 master with transparent margins and shadow
│   └── icon-composer-layers/    bg / layer-arc / layer-knob (SVG + 1024 PNG) for macOS 26 Icon Composer
├── ios/AppIcon.appiconset/      1024 single-size set: default, dark (transparent bg), tinted (grayscale)
├── android/
│   ├── res/mipmap-anydpi-v26/   ic_launcher.xml, ic_launcher_round.xml (adaptive + monochrome)
│   ├── res/mipmap-{m,h,xh,xxh,xxxh}dpi/
│   │                            ic_launcher(.png/_round), _foreground, _background, _monochrome
│   └── play-store-512.png
├── windows/
│   ├── TmpNam.ico               16 20 24 32 40 48 64 256
│   ├── png/                     the same sizes as loose PNGs
│   └── msix/Assets/             Square44x44Logo (targetsize + unplated), Square150x150Logo,
│                                Wide310x150Logo, StoreLogo, SplashScreen (scale-100, scale-200)
├── linux/
│   ├── hicolor/{16…512}x…/apps/tmp-nam.png
│   ├── hicolor/scalable/apps/tmp-nam.svg
│   ├── hicolor/symbolic/apps/tmp-nam-symbolic.svg
│   └── tmp-nam.desktop          sample entry (Icon=tmp-nam)
├── web/                         favicon.ico, favicon.svg, apple-touch-icon.png,
│                                icon-192.png, icon-512.png, icon-maskable-512.png, site.webmanifest
└── source/
    ├── svg/                     every master SVG
    ├── knob.py                  the geometry (one function draws every variant)
    └── build_icons.py           regenerates this whole pack
```

## Wiring it in, by build system

**Xcode (SwiftUI or AppKit), the main target.** Replace `Assets.xcassets/AppIcon.appiconset` with `macos/AppIcon.appiconset` (copy the folder over, keeping its name). Check the target's *App Icon* setting still reads `AppIcon`. The app is dark only, so you don't need a separate dark variant on macOS. For an iOS target, use `ios/AppIcon.appiconset`. It needs Xcode 16 or later because of the dark and tinted appearances; on older Xcode, delete the two appearance entries from its `Contents.json`.

**macOS 26 Liquid Glass (optional).** Open Icon Composer, create `AppIcon.icon`, then drop in `icon-composer-layers/bg` as the background, then `layer-arc` and `layer-knob` as two groups. Set the Dark and Tinted appearances to taste and add the `.icon` file to the target. Keep the `.appiconset` too, for macOS 15 and earlier.

**Tauri.** Run `npm run tauri icon TMP-NAM-Icons/macos/AppIcon-1024.png`. Then overwrite the generated `src-tauri/icons/icon.icns` and `icon.ico` with `macos/AppIcon.icns` and `windows/TmpNam.ico`, because those two contain the hand-tuned small sizes.

**Electron (electron-builder).** Use `build.mac.icon = TMP-NAM-Icons/macos/AppIcon.icns`, `build.win.icon = TMP-NAM-Icons/windows/TmpNam.ico` and `build.linux.icon = TMP-NAM-Icons/linux/hicolor`. With Electron Forge, set `packagerConfig.icon` to the path without its extension.

**Flutter.** Copy `macos/AppIcon.appiconset` to `macos/Runner/Assets.xcassets/AppIcon.appiconset`, `ios/AppIcon.appiconset` to `ios/Runner/Assets.xcassets/AppIcon.appiconset`, `android/res/*` into `android/app/src/main/res/` and `windows/TmpNam.ico` to `windows/runner/resources/app_icon.ico`.

**Android (native).** Copy `android/res/*` into `app/src/main/res/`, merging with the existing folders. Make sure the manifest has `android:icon="@mipmap/ic_launcher"` and `android:roundIcon="@mipmap/ic_launcher_round"`. Delete any old `ic_launcher*.webp` files so they don't shadow the new PNGs.

**Windows MSIX.** Copy `windows/msix/Assets/*` into the package's `Assets/` folder and point `Package.appxmanifest` at `Square44x44Logo`, `Square150x150Logo`, `Wide310x150Logo`, `StoreLogo` and `SplashScreen`. Set the tile background to `#1C1C1E`.

**Linux.** Install `linux/hicolor/` into `$PREFIX/share/icons/hicolor/` and `tmp-nam.desktop` into `$PREFIX/share/applications/`. If the app has a reverse-DNS app ID (Flatpak), rename `tmp-nam` everywhere, including file names and `Icon=`, to that ID.

**Web.** Copy `web/*` to the site's root and add these lines to `<head>`:
```html
<link rel="icon" href="/favicon.ico" sizes="48x48">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
<link rel="manifest" href="/site.webmanifest">
```

## Regenerating

If anything about the geometry or colours changes, edit `source/knob.py`, not the PNGs. Then run:

```
pip install playwright pillow && python -m playwright install chromium
cd TMP-NAM-Icons/source && python3 build_icons.py ../../TMP-NAM-Icons-new
```

`build_icons.py` draws every master SVG from `knob.py`, then renders every PNG, `.icns` and `.ico` with headless Chromium.

## Checks before calling it done

- On macOS, the Dock, Finder (icon and list view), ⌘-Tab and the About panel all show the knob. In list view (16 pt) it's the simplified drawing.
- No old icon is left in the asset catalog, in `res/`, or as a cached `.icns`. On macOS, run `touch` on the `.app` or log out to clear the icon cache if Finder still shows the old one.
- On Android 13 or later with themed icons on, the launcher shows the monochrome knob.
- Don't add a white background to the macOS PNGs. Their transparent corners are intended.
