# FFTARecomp for PS5

Final Fantasy Tactics Advance (USA or Europe) on the PS5: Vulkan video, audio and DualSense/gamepad input, full screen
(4K output, 1080p when the TV has no 4K mode). The GBA core is the one of the main project (`core/crates/`, copied
unchanged); on the console it runs on the interpreter, because the recompiled blocks need a C compiler at run time.

## ROM

Copy your own dump to the console:

```
/data/FFTARecomp/<any name>.gba         Final Fantasy Tactics Advance (USA or Europe)
/data/FFTARecomp/saves/ffta.sav         battery save (written by the app)
/data/FFTARecomp/settings.cfg           menu settings
/data/FFTARecomp/logs/FFTARecomp.log
```

The app creates these folders on first start and waits for the ROM if it is not there.

## Controls (the "Gamepad default" of the main README)

| DualSense (Xbox name) | Action |
|---|---|
| Cross (A) | GBA A |
| Circle (B) | GBA B |
| L2 (LT) | GBA L |
| R2 (RT) | GBA R |
| L1 (LB) | Zoom out |
| R1 (RB) | Zoom in |
| L3 | Hold fast forward (silent) |
| R3 | Toggle fast forward |
| Options (Start) | Start |
| Touchpad (Back / View) | Select |
| D-pad or left stick | Move |

| Touchpad + Options | Open the menu (the Select + Start of a gamepad; Guide is not available: the PS button belongs to the system) |

In the menu: d-pad or left stick move, Cross accepts, Circle goes back. It has Fast forward (multiplier, audio mode),
Text Bubbles, Audio (volume), Video (aspect, brightness, scaling: sharp pixels by default) and Exit. The settings are
kept in `/data/FFTARecomp/settings.cfg`. Keyboard, mouse, key remapping and ROM switching of the PC menu are not included.

## Build

Linux/WSL with `clang-18`, `ld.lld-18`, `ninja`, `python3`, `wget`, `unzip`, `make` and `rustup`.
Everything the sources need is in this folder (`core/vendor/` holds the Rust dependencies, so cargo works offline).
The console toolchain (PS5 payload SDK, SDL2, the RADV Vulkan driver and the privilege helper) is downloaded to `.deps/`
(ignored by git) by `tools/`; the RADV build comes from a GitHub release: set `RADV_RELEASE_REPO=<owner/repo>`
(with `gh` logged in) or place a prepared `.deps/` here.

```
python3 build_ps5.py all                  # core (Rust) + app (C) -> dist/PPSA98512
python3 build_ps5.py deploy --ip <ps5>    # FTP to /data/homebrew/PPSA98512
python3 build_ps5.py all --package        # also dist/PPSA98512.zip
```

The Rust core needs the nightly pinned in `core/rust-toolchain.toml` (rustup installs it): the SDK's linker rejects
native thread-local storage, so the standard library is built for `core/x86_64-ps5.json`, FreeBSD without it.

## Status

Builds and links. **Not tested on a console yet**, and the interpreter's speed on the PS5 is untested.
