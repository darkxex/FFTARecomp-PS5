/* C ABI of the FFTARecomp core (core/src/lib.rs). */
#ifndef FFTA_CORE_H
#define FFTA_CORE_H
#include <stddef.h>
#include <stdint.h>

typedef struct FftaCore FftaCore;

/* NULL when the image is not Final Fantasy Tactics Advance (USA or Europe). */
FftaCore *ffta_core_create(const uint8_t *rom, size_t len);
void ffta_core_destroy(FftaCore *core);
/* Where the menu keeps its settings (read now if the file exists). */
void ffta_core_set_config(FftaCore *core, const char *path);
/* The multiplier chosen in the menu (2, 4, 8), the scaler (0 off, 1 nearest, 2 sharp bilinear, 3 EPX) and the grid's strength. */
int ffta_core_ff_speed(FftaCore *core);
int ffta_core_scaling(FftaCore *core);
float ffta_core_grid(FftaCore *core);
int ffta_core_show_fps(FftaCore *core); /* the menu's FPS counter */
/* The pause menu of the PC build: open, navigate (0 up, 1 down, 2 left, 3 right, 4 confirm, 5 cancel;
 * returns 1 when it closed, 2 to exit the game) and draw over the last picture (1280x720 RGBA). */
void ffta_core_menu_set_open(FftaCore *core, int open);
int ffta_core_menu_is_open(FftaCore *core);
int ffta_core_menu_input(FftaCore *core, int input);
const uint8_t *ffta_core_menu_draw(FftaCore *core, int *w, int *h);
/* Where the menu keeps its settings (read now if the file exists). */
void ffta_core_set_config(FftaCore *core, const char *path);
/* The multiplier chosen in the menu (2, 4, 8), the scaler (0 off, 1 nearest, 2 sharp bilinear, 3 EPX) and the grid's strength. */
int ffta_core_ff_speed(FftaCore *core);
int ffta_core_scaling(FftaCore *core);
float ffta_core_grid(FftaCore *core);
int ffta_core_show_fps(FftaCore *core); /* the menu's FPS counter */
/* The pause menu of the PC build: open, navigate (0 up, 1 down, 2 left, 3 right, 4 confirm, 5 cancel;
 * returns 1 when it closed, 2 to exit the game) and draw over the last picture (1280x720 RGBA). */
void ffta_core_menu_set_open(FftaCore *core, int open);
int ffta_core_menu_is_open(FftaCore *core);
int ffta_core_menu_input(FftaCore *core, int input);
const uint8_t *ffta_core_menu_draw(FftaCore *core, int *w, int *h);
/* A layer of the UI (dialogues, menus, panels), as the PC draws it over the picture: a 480x160 RGBA picture
 * (alpha 0 where there is nothing), where it goes and the area it is clipped to, in output pixels. */
typedef struct
{
    const uint8_t *rgba;
    float rect[4];
    float clip[4];
} FftaUiLayer;
/* The layers for the picture drawn at (x, y) with width w (plat_game_rect). Fills out[2]; returns how many. */
int ffta_core_ui_layers(FftaCore *core, float x, float y, float w, FftaUiLayer *out);
/* Zoom out by steps (negative: in); returns the level 0..6. */
int ffta_core_zoom(FftaCore *core, int steps);
/* Runs 1..8 frames. keys is KEYINPUT: a clear bit is a pressed button. */
void ffta_core_run(FftaCore *core, uint16_t keys, int frames);
/* XRGB8888 picture of the last frame, cropped to the zoom; pitch in bytes. */
const uint32_t *ffta_core_video(FftaCore *core, int *w, int *h, size_t *pitch);
/* 48 kHz stereo frames made by the last run. */
const int16_t *ffta_core_audio(FftaCore *core, size_t *frames);
const uint8_t *ffta_core_save(FftaCore *core, size_t *len);
void ffta_core_load_save(FftaCore *core, const uint8_t *data, size_t len);
#endif
