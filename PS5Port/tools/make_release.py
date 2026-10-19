#!/usr/bin/env python3
"""Build everything (core + app + zip) and deploy it to the console over FTP.

    python3 tools/make_release.py                   # uses 192.168.1.89
    python3 tools/make_release.py --ip 192.168.1.50
"""
import argparse
import subprocess
import sys
from pathlib import Path

DEFAULT_IP = '192.168.1.89'
BUILD = Path(__file__).resolve().parent.parent / 'build_ps5.py'


def run(*args):
    cmd = [sys.executable, str(BUILD), *args]
    print('+', ' '.join(cmd), flush=True)
    subprocess.run(cmd, check=True)


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--ip', default=DEFAULT_IP, help='console address (default %(default)s)')
    p.add_argument('--ftp-port', type=int, default=2121)
    p.add_argument('--no-package', action='store_true', help='skip dist/PPSA98512.zip')
    a = p.parse_args()

    run('all', *([] if a.no_package else ['--package']))
    run('deploy', '--ip', a.ip, '--ftp-port', str(a.ftp_port))
    print(f'Built and deployed to {a.ip}.')


if __name__ == '__main__':
    try:
        main()
    except subprocess.CalledProcessError as e:
        sys.exit(e.returncode)
