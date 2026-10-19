#!/usr/bin/env python3
"""Build FFTARecomp for PS5.

  core    build the GBA core (Rust, core/) as a static library for the console
  host    build the PS5 app (C, app/) and link the core into it
  deploy  upload dist/PPSA98512 to a console over FTP (--ip)
  all     core + host

Output: dist/PPSA98512 (and dist/PPSA98512.zip with --package).
"""
import argparse
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import sys
import zipfile

HERE = Path(__file__).resolve().parent
BUILD = HERE / 'build'
DIST = HERE / 'dist'
TITLE_ID = 'PPSA98512'
# The console's system is FreeBSD-based, but its linker refuses native thread-local storage:
# the target is FreeBSD without it (the standard library is built for it, see core/x86_64-ps5.json).
RUST_TARGET = 'x86_64-ps5'
CORE_LIB = f'build/core/{RUST_TARGET}/release/libffta_ps5_core.a'


def run(cmd, **kw):
    r = subprocess.run([str(c) for c in cmd], **kw)
    if r.returncode:
        sys.exit(f'{Path(str(cmd[0])).name} failed ({r.returncode})')


def build_core():
    env = dict(os.environ, PATH=os.environ['PATH'] + f":{Path.home()}/.cargo/bin")
    run(['cargo', 'build', '--release', '--offline', '-Zbuild-std=std,panic_abort', '-Zjson-target-spec',
         '--target', 'x86_64-ps5.json'], cwd=HERE / 'core', env=env)


def sync_content_version():
    version = re.search(r'#define FFTA_VERSION "(\d+)\.(\d+)\.(\d+)"', (HERE / 'app/FFTA.h').read_text())
    content = '%02d.%03d.%03d' % tuple(int(v) for v in version.groups())
    param = HERE / 'sce_sys/param.json'
    text = param.read_text()
    new = re.sub(r'"contentVersion": "[^"]*"', f'"contentVersion": "{content}"', text)
    if new != text:
        param.write_text(new)


MESSAGES = {
    'need-rom': ('ROM NEEDED', ['Final Fantasy Tactics Advance (USA or Europe), .gba', 'Copy it (any name, .gba) to:', '/data/FFTARecomp',
                                'The game starts by itself once it is there.']),
    'wrong-rom': ('THIS ROM IS NOT SUPPORTED', ['Only Final Fantasy Tactics Advance (USA or Europe) works.', 'Replace the .gba file in:',
                                                '/data/FFTARecomp']),
}


TOASTS = {
    'toast-menu': ['Touchpad + Options: menu', 'L1 / R1: zoom out / in', 'L3: hold fast forward', 'R3: toggle fast forward'],
}


def render_toasts(out):
    """Small boxes the app draws over the game for a few seconds: RGBA with a w, h header."""
    from PIL import Image, ImageDraw, ImageFont
    font = ImageFont.truetype(str(HERE / 'assets/fonts/DejaVuSans-Bold.ttf'), 40)
    for name, lines in TOASTS.items():
        w, h = 760, 40 + 62 * len(lines)
        panel = Image.new('RGBA', (w, h), (0, 0, 0, 0))
        d = ImageDraw.Draw(panel)
        d.rounded_rectangle((0, 0, w - 1, h - 1), 24, fill=(14, 16, 36, 225), outline=(130, 150, 255, 255), width=3)
        for i, line in enumerate(lines):
            d.text((w // 2, 20 + 62 * i + 31), line, font=font, fill=(235, 238, 255, 255), anchor='mm')
        (out / f'{name}.rgba').write_bytes(struct.pack('<II', w, h) + panel.tobytes())


def render_messages(out):
    """Full-screen notices the app draws when it has nothing to play: RGBA with a w, h header."""
    from PIL import Image, ImageDraw, ImageFont
    font_path = str(HERE / 'assets/fonts/DejaVuSans-Bold.ttf')
    title_font, body_font = ImageFont.truetype(font_path, 60), ImageFont.truetype(font_path, 36)
    for name, (title, lines) in MESSAGES.items():
        w, h = 1500, 150 + 62 * len(lines)
        panel = Image.new('RGBA', (w, h), (0, 0, 0, 0))
        d = ImageDraw.Draw(panel)
        d.rounded_rectangle((0, 0, w - 1, h - 1), 28, fill=(14, 16, 36, 235), outline=(130, 150, 255, 255), width=3)
        d.text((w // 2, 70), title, font=title_font, fill=(255, 255, 255, 255), anchor='mm')
        for i, line in enumerate(lines):
            colour = (255, 214, 102, 255) if line.startswith('/') else (214, 220, 255, 255)
            d.text((w // 2, 150 + 62 * i + 20), line, font=body_font, fill=colour, anchor='mm')
        (out / f'{name}.rgba').write_bytes(struct.pack('<II', w, h) + panel.tobytes())


def build_host():
    sync_content_version()
    assets = BUILD / 'assets'
    shutil.rmtree(assets, ignore_errors=True)
    assets.mkdir(parents=True)
    render_messages(assets)
    render_toasts(assets)
    if not (HERE / CORE_LIB).is_file():
        build_core()
    env = dict(os.environ,
               APP_VULKAN='1', APP_SOURCE_DIR='app', APP_PARAM='sce_sys/param.json', APP_SCE_SYS='sce_sys',
               APP_ASSETS='build/assets', APP_LAPY_HELPER='1', APP_ROOT_FILES='', APP_RUNTIME_MODULES='',
               APP_STATIC_ARCHIVES=CORE_LIB, APP_DEFINITIONS='FFTA_VULKAN=1',
               APP_INCLUDE_PATHS='app app/platform core .deps/native/radv-release/include',
               APP_WRAP_SYMBOLS='malloc free calloc realloc reallocf memalign posix_memalign aligned_alloc malloc_usable_size',
               PACBREW_PACKAGES='sdl2', PACBREW_INCLUDE_PATHS='', PACBREW_STATIC_ARCHIVES='')
    run(['bash', HERE / 'tools/fetch-radv.sh'], env=env)
    run(['bash', HERE / 'tools/build.sh', 'Folder'], cwd=HERE, env=env)
    print('app:', DIST / TITLE_ID)


def package():
    app = DIST / TITLE_ID
    archive = DIST / f'{TITLE_ID}.zip'
    archive.unlink(missing_ok=True)
    with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED, compresslevel=9) as zf:
        for path in sorted(p for p in app.rglob('*') if p.is_file()):
            zf.write(path, Path(TITLE_ID) / path.relative_to(app))
    print('package:', archive)


def deploy(host, port):
    from ftplib import FTP
    app = DIST / TITLE_ID
    files = sorted(p for p in app.rglob('*') if p.is_file())
    last = [app / 'eboot.bin', app / 'sce_sys/param.json']
    files = [p for p in files if p not in last] + last
    with FTP() as ftp:
        ftp.connect(host, port, timeout=15)
        ftp.login('anonymous', 'ffta')
        made = set()

        def mkdirs(path):
            parts = ''
            for part in path.strip('/').split('/'):
                parts += '/' + part
                if parts not in made:
                    try:
                        ftp.mkd(parts)
                    except Exception:
                        pass
                    made.add(parts)
        for p in files:
            remote = f'/data/homebrew/{TITLE_ID}/' + p.relative_to(app).as_posix()
            mkdirs(os.path.dirname(remote))
            # upload under a temporary name and rename: a running eboot.bin cannot be overwritten
            temporary = os.path.dirname(remote) + '/.' + os.path.basename(remote) + '.upload'
            try:
                ftp.sendcmd('DELE ' + temporary)
            except Exception:
                pass
            with open(p, 'rb') as f:
                ftp.storbinary('STOR ' + temporary, f, blocksize=256 * 1024)
            try:
                ftp.sendcmd('DELE ' + remote)
            except Exception:
                pass
            ftp.rename(temporary, remote)
            print('uploaded', remote)
        ftp.quit()


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('step', nargs='?', default='all', choices=['core', 'host', 'all', 'deploy'])
    p.add_argument('--ip', help='console address for deploy (FTP)')
    p.add_argument('--ftp-port', type=int, default=2121)
    p.add_argument('--package', action='store_true', help='write dist/PPSA98512.zip after building')
    args = p.parse_args()
    if args.step in ('core', 'all'):
        build_core()
    if args.step in ('host', 'all'):
        build_host()
        if args.package:
            package()
    if args.step == 'deploy':
        if not args.ip:
            p.error('deploy needs --ip')
        deploy(args.ip, args.ftp_port)


if __name__ == '__main__':
    main()
