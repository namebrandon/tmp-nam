"""Builds every platform's icon files from the master SVGs (knob.py). Run: python3 build_icons.py <out_dir>"""
import os, sys, json, shutil, io
from playwright.sync_api import sync_playwright
from PIL import Image
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from knob import FILES, svg, knob
OUT = sys.argv[1] if len(sys.argv) > 1 else 'TMP-NAM-Icons'
SRC = {k: v for k, v in FILES.items()}
SRC['mark-unplated.svg'] = svg(knob(c=(512, 522), s=1.62))
SRC['ios-dark-transparent.svg'] = svg(knob(c=(512, 522), s=1.2), glow=26)

def p(*a):
    f = os.path.join(OUT, *a); os.makedirs(os.path.dirname(f), exist_ok=True); return f

def main():
    if os.path.exists(OUT): shutil.rmtree(OUT)
    for k, v in SRC.items(): open(p('source', 'svg', k), 'w').write(v)
    with sync_playwright() as pw:
        b = pw.chromium.launch(); pg = b.new_page()
        def html(body, w, h, path, opaque=False):
            pg.set_viewport_size({'width': w, 'height': h})
            pg.set_content(f'<html><body style="margin:0;width:{w}px;height:{h}px;overflow:hidden">{body}</body></html>')
            pg.wait_for_timeout(30)
            data = pg.screenshot(omit_background=True, clip={'x': 0, 'y': 0, 'width': w, 'height': h})
            im = Image.open(io.BytesIO(data)).convert('RGBA')
            if opaque: im = im.convert('RGB')
            im.save(path); return path
        def r(name, n, path, opaque=False):
            return html(f'<div style="width:{n}px;height:{n}px">{SRC[name].replace("<svg ", f"<svg width=\"{n}\" height=\"{n}\" ", 1)}</div>', n, n, path, opaque)
        def tile(name, n, w, h, path, bgc='transparent'):
            s = SRC[name].replace('<svg ', f'<svg width="{n}" height="{n}" ', 1)
            return html(f'<div style="width:{w}px;height:{h}px;display:flex;align-items:center;justify-content:center;background:{bgc}">{s}</div>', w, h, path)
        def masked(layers, n, radius, path):
            imgs = ''.join(f'<div style="position:absolute;inset:0">{SRC[l].replace("<svg ", f"<svg width=\"{n}\" height=\"{n}\" ", 1)}</div>' for l in layers)
            return html(f'<div style="position:relative;width:{n}px;height:{n}px;border-radius:{radius};overflow:hidden">{imgs}</div>', n, n, path)

        # ---------- macOS ----------
        mac = []
        for pt in (16, 32, 128, 256, 512):
            for sc in (1, 2):
                px = pt * sc; fn = f'icon_{pt}x{pt}' + ('@2x' if sc == 2 else '') + '.png'
                r('macos-small.svg' if px <= 32 else 'macos.svg', px, p('macos', 'AppIcon.appiconset', fn))
                mac.append({'filename': fn, 'idiom': 'mac', 'scale': f'{sc}x', 'size': f'{pt}x{pt}'})
        json.dump({'images': mac, 'info': {'author': 'xcode', 'version': 1}}, open(p('macos', 'AppIcon.appiconset', 'Contents.json'), 'w'), indent=2)
        d = os.path.join(OUT, 'macos', 'AppIcon.appiconset')
        big = Image.open(os.path.join(d, 'icon_512x512@2x.png'))
        extra = [Image.open(os.path.join(d, f)) for f in ('icon_16x16.png', 'icon_16x16@2x.png', 'icon_128x128.png', 'icon_256x256.png', 'icon_512x512.png', 'icon_32x32@2x.png')]
        big.save(p('macos', 'AppIcon.icns'), append_images=extra)
        r('macos.svg', 1024, p('macos', 'AppIcon-1024.png'))
        for l in ('bg.svg', 'layer-arc.svg', 'layer-knob.svg'):
            shutil.copy(p('source', 'svg', l), p('macos', 'icon-composer-layers', l))
            r(l, 1024, p('macos', 'icon-composer-layers', l.replace('.svg', '.png')))

        # ---------- iOS ----------
        r('full.svg', 1024, p('ios', 'AppIcon.appiconset', 'AppIcon-1024.png'), opaque=True)
        r('ios-dark-transparent.svg', 1024, p('ios', 'AppIcon.appiconset', 'AppIcon-1024-dark.png'))
        r('ios-tinted.svg', 1024, p('ios', 'AppIcon.appiconset', 'AppIcon-1024-tinted.png'), opaque=True)
        ios = {'images': [
            {'filename': 'AppIcon-1024.png', 'idiom': 'universal', 'platform': 'ios', 'size': '1024x1024'},
            {'appearances': [{'appearance': 'luminosity', 'value': 'dark'}], 'filename': 'AppIcon-1024-dark.png', 'idiom': 'universal', 'platform': 'ios', 'size': '1024x1024'},
            {'appearances': [{'appearance': 'luminosity', 'value': 'tinted'}], 'filename': 'AppIcon-1024-tinted.png', 'idiom': 'universal', 'platform': 'ios', 'size': '1024x1024'}],
            'info': {'author': 'xcode', 'version': 1}}
        json.dump(ios, open(p('ios', 'AppIcon.appiconset', 'Contents.json'), 'w'), indent=2)

        # ---------- Android ----------
        for q, f in (('mdpi', 1), ('hdpi', 1.5), ('xhdpi', 2), ('xxhdpi', 3), ('xxxhdpi', 4)):
            L = int(108 * f); n = int(48 * f)
            r('bg.svg', L, p('android', 'res', f'mipmap-{q}', 'ic_launcher_background.png'), opaque=True)
            r('mark-android.svg', L, p('android', 'res', f'mipmap-{q}', 'ic_launcher_foreground.png'))
            r('mark-mono-android.svg', L, p('android', 'res', f'mipmap-{q}', 'ic_launcher_monochrome.png'))
            # legacy: 108dp art cropped to the 72dp viewport, then shaped
            big = int(n * 1.5)
            html(f'<div style="position:relative;width:{n}px;height:{n}px;border-radius:22%;overflow:hidden">' + ''.join(f'<div style="position:absolute;left:{-(big-n)/2}px;top:{-(big-n)/2}px">{SRC[l].replace("<svg ", f"<svg width=\"{big}\" height=\"{big}\" ", 1)}</div>' for l in ('bg.svg', 'mark-android.svg')) + '</div>', n, n, p('android', 'res', f'mipmap-{q}', 'ic_launcher.png'))
            html(f'<div style="position:relative;width:{n}px;height:{n}px;border-radius:50%;overflow:hidden">' + ''.join(f'<div style="position:absolute;left:{-(big-n)/2}px;top:{-(big-n)/2}px">{SRC[l].replace("<svg ", f"<svg width=\"{big}\" height=\"{big}\" ", 1)}</div>' for l in ('bg.svg', 'mark-android.svg')) + '</div>', n, n, p('android', 'res', f'mipmap-{q}', 'ic_launcher_round.png'))
        xml = '''<?xml version="1.0" encoding="utf-8"?>
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
    <background android:drawable="@mipmap/ic_launcher_background" />
    <foreground android:drawable="@mipmap/ic_launcher_foreground" />
    <monochrome android:drawable="@mipmap/ic_launcher_monochrome" />
</adaptive-icon>
'''
        open(p('android', 'res', 'mipmap-anydpi-v26', 'ic_launcher.xml'), 'w').write(xml)
        open(p('android', 'res', 'mipmap-anydpi-v26', 'ic_launcher_round.xml'), 'w').write(xml)
        r('full.svg', 512, p('android', 'play-store-512.png'), opaque=True)

        # ---------- Windows ----------
        ico = []
        for n in (16, 20, 24, 32, 40, 48, 64, 256):
            ico.append(r('windows-small.svg' if n <= 32 else 'windows.svg', n, p('windows', 'png', f'icon-{n}.png')))
        ims = [Image.open(f) for f in ico]
        ims[-1].save(p('windows', 'TmpNam.ico'), sizes=[i.size for i in ims], append_images=ims[:-1])
        A = ('windows', 'msix', 'Assets')
        for n in (16, 24, 32, 48, 256):
            src = 'windows-small.svg' if n <= 32 else 'windows.svg'
            r(src, n, p(*A, f'Square44x44Logo.targetsize-{n}.png'))
            r('mark-unplated.svg', n, p(*A, f'Square44x44Logo.targetsize-{n}_altform-unplated.png'))
            r('mark-unplated.svg', n, p(*A, f'Square44x44Logo.targetsize-{n}_altform-lightunplated.png'))
        for sc, f in ((100, 1), (200, 2)):
            r('windows.svg' if 44 * f > 32 else 'windows-small.svg', 44 * f, p(*A, f'Square44x44Logo.scale-{sc}.png'))
            tile('windows.svg', int(72 * f), 150 * f, 150 * f, p(*A, f'Square150x150Logo.scale-{sc}.png'))
            tile('windows.svg', int(72 * f), 310 * f, 150 * f, p(*A, f'Wide310x150Logo.scale-{sc}.png'))
            r('windows.svg', 50 * f, p(*A, f'StoreLogo.scale-{sc}.png'))
            tile('windows.svg', int(160 * f), 620 * f, 300 * f, p(*A, f'SplashScreen.scale-{sc}.png'))

        # ---------- Linux ----------
        for n in (16, 22, 24, 32, 48, 64, 128, 256, 512):
            r('gnome-small.svg' if n <= 32 else 'gnome.svg', n, p('linux', 'hicolor', f'{n}x{n}', 'apps', 'tmp-nam.png'))
        shutil.copy(p('source', 'svg', 'gnome.svg'), p('linux', 'hicolor', 'scalable', 'apps', 'tmp-nam.svg'))
        shutil.copy(p('source', 'svg', 'symbolic-dark.svg'), p('linux', 'hicolor', 'symbolic', 'apps', 'tmp-nam-symbolic.svg'))
        open(p('linux', 'tmp-nam.desktop'), 'w').write('[Desktop Entry]\nType=Application\nName=TMP NAM\nComment=Manage NAM captures on the Tone Master Pro\nExec=tmp-nam\nIcon=tmp-nam\nCategories=AudioVideo;Audio;\nTerminal=false\n')

        # ---------- Web ----------
        fav = [r('favicon.svg', n, p('web', f'_favicon-{n}.png')) for n in (16, 32, 48)]
        fi = [Image.open(f) for f in fav]
        fi[-1].save(p('web', 'favicon.ico'), sizes=[i.size for i in fi], append_images=fi[:-1])
        for f in fav: os.remove(f)
        shutil.copy(p('source', 'svg', 'favicon.svg'), p('web', 'favicon.svg'))
        r('full.svg', 180, p('web', 'apple-touch-icon.png'), opaque=True)
        r('full.svg', 192, p('web', 'icon-192.png'))
        r('full.svg', 512, p('web', 'icon-512.png'))
        r('maskable.svg', 512, p('web', 'icon-maskable-512.png'), opaque=True)
        json.dump({'name': 'TMP NAM', 'short_name': 'TMP NAM', 'theme_color': '#1C1C1E', 'background_color': '#1C1C1E', 'display': 'standalone',
                   'icons': [{'src': '/icon-192.png', 'sizes': '192x192', 'type': 'image/png'},
                             {'src': '/icon-512.png', 'sizes': '512x512', 'type': 'image/png'},
                             {'src': '/icon-maskable-512.png', 'sizes': '512x512', 'type': 'image/png', 'purpose': 'maskable'}]},
                  open(p('web', 'site.webmanifest'), 'w'), indent=2)
        b.close()
    for f in ('knob.py', 'build_icons.py'):
        shutil.copy(os.path.join(os.path.dirname(os.path.abspath(__file__)), f), p('source', f))

if __name__ == '__main__':
    main()
