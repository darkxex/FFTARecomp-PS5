/* FFTARecomp for PS5: runs the GBA core (core/, Rust, interpreter) and serves it
 * with the platform layer: Vulkan presentation at full screen, audio and pads.
 *
 *   /data/FFTARecomp/*.gba    the Final Fantasy Tactics Advance (USA or Europe) cartridge, any file name
 *   /data/FFTARecomp/saves/   battery save (ffta.sav)
 *   /data/FFTARecomp/logs/    FFTARecomp.log
 *
 * Controls follow the "Gamepad default" table of the README (Xbox names on a
 * DualSense: A = Cross, B = Circle, LT/RT = L2/R2, LB/RB = L1/R1, View = touchpad).
 * The menu (Esc on PC) opens with touchpad + Options, the Select + Start of a gamepad. */
#include <dirent.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "platform/platform.h"
#include "platform/ps5_crash.h"
#include "platform/vk/vk_present.h"
#include "../core/ffta_core.h"

#define FFTA_TITLE_ID "PPSA98512"
#define DATA_ROOT "/data/FFTARecomp"
#define ROM_DIR DATA_ROOT
#define SAVE_DIR DATA_ROOT "/saves"
#define SAVE_PATH SAVE_DIR "/ffta.sav"
#define LOG_DIR DATA_ROOT "/logs"
#define CONFIG_PATH DATA_ROOT "/settings.cfg"
#define AUDIO_RATE 48000
#define FRAME_US 16743 /* 280896 cycles at 16.78 MHz: 59.7275 Hz */
#define STICK_DEADZONE 16000

int sceSystemServiceLoadExec(const char *path, const char **arguments);
int ps5_klog_capture_stderr(const char *prefix);

/* ---- notices: pictures made at build time (build_ps5.py), assets/<name>.rgba with a w, h header ---- */

typedef struct
{
    int tex, w, h;
} Notice;

static Notice LoadNotice(const char *name)
{
    static const char *const roots[] = {"/app0/assets", "/system_ex/app/" FFTA_TITLE_ID "/assets",
                                        "/data/homebrew/" FFTA_TITLE_ID "/assets"};
    Notice n = {-1, 0, 0};
    for (size_t i = 0; i < sizeof(roots) / sizeof(roots[0]) && n.tex < 0; ++i)
    {
        char path[512];
        snprintf(path, sizeof(path), "%s/%s.rgba", roots[i], name);
        FILE *f = fopen(path, "rb");
        if (!f)
            continue;
        uint32_t size[2];
        if (fread(size, sizeof(size), 1, f) == 1 && size[0] && size[1] && size[0] <= 4096 && size[1] <= 4096)
        {
            size_t bytes = (size_t)size[0] * size[1] * 4;
            uint8_t *pixels = malloc(bytes);
            if (pixels && fread(pixels, 1, bytes, f) == bytes)
            {
                n.tex = vkp_ui_texture_create(pixels, (int)size[0], (int)size[1], true, false, true);
                n.w = (int)size[0];
                n.h = (int)size[1];
            }
            free(pixels);
        }
        fclose(f);
    }
    return n;
}

/* One frame of a notice, centred and scaled to the screen (1500 px wide at 1920 px). */
static void DrawNotice(const Notice *n)
{
    plat_begin_frame(0xFF0A0A18u);
    if (n->tex >= 0)
    {
        const uint32_t white = 0xFFFFFFFFu;
        float k = plat_width() / 1920.0f, dw = (float)n->w * k, dh = (float)n->h * k;
        float x0 = (plat_width() - dw) / 2, y0 = (plat_height() - dh) / 2;
        VkpUiVertex q[6] = {{x0, y0, 0, 0, white},      {x0 + dw, y0, 1, 0, white},      {x0, y0 + dh, 0, 1, white},
                            {x0 + dw, y0, 1, 0, white}, {x0, y0 + dh, 0, 1, white},      {x0 + dw, y0 + dh, 1, 1, white}};
        vkp_ui_blend(true);
        vkp_ui_draw(n->tex, q, 6);
    }
    plat_end_frame();
}

/* Shows a notice for `frames` screen refreshes, keeping the controllers and system events alive. */
static void ShowNotice(const Notice *n, int frames)
{
    for (int i = 0; i < frames; ++i)
    {
        PadState pads[FFTA_MAX_PADS];
        bool quit = false;
        plat_poll(pads, &quit);
        DrawNotice(n);
    }
}

/* A box over the game for five seconds (assets/<name>.rgba), fading out at the end. */
static Notice sToast = {-1, 0, 0};
static uint64_t sToastUntil;

static void ShowToast(const char *name)
{
    if (sToast.tex >= 0)
        vkp_ui_texture_free(sToast.tex);
    sToast = LoadNotice(name);
    sToastUntil = plat_ticks_us() + 5000000u;
}

static void DrawToast(void)
{
    uint64_t now = plat_ticks_us();
    if (sToast.tex < 0)
        return;
    if (now >= sToastUntil)
    {
        vkp_ui_texture_free(sToast.tex);
        sToast.tex = -1;
        return;
    }
    uint64_t left = sToastUntil - now;
    uint32_t alpha = left < 400000u ? (uint32_t)(left * 255u / 400000u) : 255u;
    uint32_t tint = (alpha << 24) | 0xFFFFFFu;
    float k = plat_width() / 1920.0f, dw = (float)sToast.w * k, dh = (float)sToast.h * k;
    float x0 = (plat_width() - dw) / 2, y0 = plat_height() - dh - 90.0f * k;
    VkpUiVertex q[6] = {{x0, y0, 0, 0, tint},      {x0 + dw, y0, 1, 0, tint},      {x0, y0 + dh, 0, 1, tint},
                        {x0 + dw, y0, 1, 0, tint}, {x0, y0 + dh, 0, 1, tint},      {x0 + dw, y0 + dh, 1, 1, tint}};
    vkp_ui_clip(0, 0, 0, 0);
    vkp_ui_blend(true);
    vkp_ui_draw(sToast.tex, q, 6);
}

/* The frame counter at the top left: a 3x5 font for digits and "FPS" drawn with filled rectangles. */
static const struct { char c; uint8_t rows[5]; } kGlyphs[] = {
    {'0', {7, 5, 5, 5, 7}}, {'1', {2, 6, 2, 2, 7}}, {'2', {7, 1, 7, 4, 7}}, {'3', {7, 1, 7, 1, 7}},
    {'4', {5, 5, 7, 1, 1}}, {'5', {7, 4, 7, 1, 7}}, {'6', {7, 4, 7, 5, 7}}, {'7', {7, 1, 1, 1, 1}},
    {'8', {7, 5, 7, 5, 7}}, {'9', {7, 5, 7, 1, 7}}, {'F', {7, 4, 6, 4, 4}}, {'P', {7, 5, 7, 4, 4}},
    {'S', {7, 4, 7, 1, 7}},
};

static void DrawFps(int fps)
{
    char text[16];
    snprintf(text, sizeof(text), "%d FPS", fps);
    int unit = plat_height() / 135; /* a font pixel: 16 px on a 4K screen */
    if (unit < 2)
        unit = 2;
    int pad = unit * 2, x = unit * 3, y = unit * 3;
    int width = (int)strlen(text) * 4 * unit - unit;
    plat_fill_rect(x - pad, y - pad, width + 2 * pad, 5 * unit + 2 * pad, 0xB0000000u);
    for (const char *c = text; *c; ++c, x += 4 * unit)
        for (size_t g = 0; g < sizeof(kGlyphs) / sizeof(kGlyphs[0]); ++g)
            if (kGlyphs[g].c == *c)
                for (int row = 0; row < 5; ++row)
                    for (int col = 0; col < 3; ++col)
                        if (kGlyphs[g].rows[row] >> (2 - col) & 1)
                            plat_fill_rect(x + col * unit, y + row * unit, unit, unit, 0xFFFFFFFFu);
}

/* The UI layers over the picture (dialogues, menus, panels): each is a 480x160 texture drawn with nearest sampling. */
static void DrawUiLayers(FftaCore *core)
{
    static int tex[2] = {-1, -1};
    FftaUiLayer layers[2];
    int gx, gy, gw, gh;
    plat_game_rect(&gx, &gy, &gw, &gh);
    int n = ffta_core_ui_layers(core, (float)gx, (float)gy, (float)gw, layers);
    for (int i = 0; i < n; ++i)
    {
        if (tex[i] < 0)
            tex[i] = vkp_ui_texture_create(layers[i].rgba, 480, 160, false, false, true);
        else
            vkp_ui_texture_update(tex[i], layers[i].rgba);
        if (tex[i] < 0)
            continue;
        const uint32_t white = 0xFFFFFFFFu;
        float x0 = layers[i].rect[0], y0 = layers[i].rect[1], x1 = x0 + layers[i].rect[2], y1 = y0 + layers[i].rect[3];
        VkpUiVertex q[6] = {{x0, y0, 0, 0, white}, {x1, y0, 1, 0, white}, {x0, y1, 0, 1, white},
                            {x1, y0, 1, 0, white}, {x0, y1, 0, 1, white}, {x1, y1, 1, 1, white}};
        vkp_ui_clip((int)layers[i].clip[0], (int)layers[i].clip[1], (int)layers[i].clip[2], (int)layers[i].clip[3]);
        vkp_ui_blend(true);
        vkp_ui_draw(tex[i], q, 6);
    }
    vkp_ui_clip(0, 0, 0, 0);
}

/* ---- files ---------------------------------------------------------------------- */

static uint8_t *ReadFile(const char *path, size_t *size)
{
    FILE *f = fopen(path, "rb");
    if (!f)
        return NULL;
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    uint8_t *data = n > 0 ? malloc((size_t)n) : NULL;
    if (data && fread(data, 1, (size_t)n, f) != (size_t)n)
    {
        free(data);
        data = NULL;
    }
    fclose(f);
    *size = data ? (size_t)n : 0;
    return data;
}

static bool FindRom(char *out, size_t size)
{
    static int logged;
    bool log = logged++ < 3; /* the first passes only: the notice loop calls this every second */
    DIR *d = opendir(ROM_DIR);
    if (!d)
    {
        if (log)
            FFTA_log("opendir(%s) failed: errno %d (%s)", ROM_DIR, errno, strerror(errno));
        return false;
    }
    bool found = false;
    struct dirent *e;
    int entries = 0;
    while ((e = readdir(d)))
    {
        ++entries;
        const char *ext = strrchr(e->d_name, '.');
        bool match = ext && !str_icmp(ext, ".gba");
        if (match && !found)
        {
            snprintf(out, size, "%s/%s", ROM_DIR, e->d_name);
            found = true;
        }
    }
    closedir(d);
    if (log)
        FFTA_log("%s: %d entries, cartridge %s", ROM_DIR, entries, found ? out : "not found");
    return found;
}

static uint32_t Hash(const uint8_t *p, size_t n)
{
    uint32_t h = 2166136261u;
    while (n--)
        h = (h ^ *p++) * 16777619u;
    return h;
}

static uint32_t sSaveHash;

static void WriteSave(FftaCore *core, bool force)
{
    size_t n;
    const uint8_t *data = ffta_core_save(core, &n);
    if (!data || !n)
        return;
    uint32_t h = Hash(data, n);
    if (!force && h == sSaveHash)
        return;
    char tmp[] = SAVE_PATH ".tmp";
    FILE *f = fopen(tmp, "wb");
    if (!f)
        return;
    bool ok = fwrite(data, 1, n, f) == n;
    fclose(f);
    if (ok)
    {
        rename(tmp, SAVE_PATH);
        sSaveHash = h;
    }
}

/* ---- input ---------------------------------------------------------------------- */

/* KEYINPUT bits: A B Select Start Right Left Up Down R L (a clear bit is pressed) */
enum { K_A = 1 << 0, K_B = 1 << 1, K_SELECT = 1 << 2, K_START = 1 << 3, K_RIGHT = 1 << 4, K_LEFT = 1 << 5,
       K_UP = 1 << 6, K_DOWN = 1 << 7, K_R = 1 << 8, K_L = 1 << 9 };

static uint16_t GbaKeys(const PadState *p)
{
    uint32_t b = p->buttons, k = 0;
    if (b & BIT(BTN_CROSS))   k |= K_A;      /* A  -> GBA A */
    if (b & BIT(BTN_CIRCLE))  k |= K_B;      /* B  -> GBA B */
    if (b & BIT(BTN_L2))      k |= K_L;      /* LT -> GBA L */
    if (b & BIT(BTN_R2))      k |= K_R;      /* RT -> GBA R */
    if (b & BIT(BTN_START))   k |= K_START;  /* Start / Menu */
    if (b & (BIT(BTN_MENU) | BIT(BTN_SELECT))) k |= K_SELECT; /* Back / View: the touchpad */
    if (b & BIT(BTN_UP) || p->ly < -STICK_DEADZONE)    k |= K_UP;    /* D-pad or left stick */
    if (b & BIT(BTN_DOWN) || p->ly > STICK_DEADZONE)   k |= K_DOWN;
    if (b & BIT(BTN_LEFT) || p->lx < -STICK_DEADZONE)  k |= K_LEFT;
    if (b & BIT(BTN_RIGHT) || p->lx > STICK_DEADZONE)  k |= K_RIGHT;
    return (uint16_t)(~k & 0x3ff);
}

static void Exit(int code)
{
    plat_audio_close();
    const char *none[1] = {NULL};
    sceSystemServiceLoadExec("exit", none);
    _exit(code);
}

int main(void)
{
    mkdir(DATA_ROOT, 0777);
    mkdir(SAVE_DIR, 0777);
    mkdir(LOG_DIR, 0777);

    char error[200] = "";
    bool unlocked = plat_prepare_storage(error, sizeof(error));
    plat_want_output(2); /* 4K, 1080p when the TV has no 4K mode */
    plat_want_high_refresh(false);
    if (!plat_init())
    {
        char msg[300];
        snprintf(msg, sizeof(msg), "FFTARecomp: %s", plat_init_error());
        plat_notify(msg);
        for (;;)
            plat_sleep_us(1000000);
    }
    mkdir(DATA_ROOT, 0777);
    mkdir(SAVE_DIR, 0777);
    mkdir(LOG_DIR, 0777);
    FFTA_log_open(LOG_DIR "/FFTARecomp.log");
    ps5_crash_install(LOG_DIR "/FFTARecomp.log");
    ps5_klog_capture_stderr("");
    FFTA_log("FFTARecomp starting; /data %s (%s)", unlocked ? "unlocked" : "sandboxed", error);
    FFTA_log("screen: %s", plat_screen_info());

    /* Without a cartridge the app stays open and says what is needed; it starts once the file is there. */
    char rom_path[FFTA_PATH_MAX], rejected[FFTA_PATH_MAX] = "";
    FftaCore *core = NULL;
    Notice need = LoadNotice("need-rom"), wrong = LoadNotice("wrong-rom");
    bool told = false;
    while (!core)
    {
        if (FindRom(rom_path, sizeof(rom_path)) && strcmp(rom_path, rejected))
        {
            size_t size;
            uint8_t *rom = ReadFile(rom_path, &size);
            FFTA_log("read %s: %s (%zu bytes)", rom_path, rom ? "ok" : "failed", rom ? size : (size_t)0);
            core = rom ? ffta_core_create(rom, size) : NULL;
            free(rom);
            if (!core)
            {
                FFTA_log("%s is not Final Fantasy Tactics Advance (USA or Europe)", rom_path);
                snprintf(rejected, sizeof(rejected), "%s", rom_path);
            }
        }
        if (core)
            break;
        if (!told && !rejected[0])
        {
            FFTA_log("no cartridge found in " ROM_DIR ": waiting for one");
            told = true;
        }
        ShowNotice(rejected[0] ? &wrong : &need, 60);
    }
    FFTA_log("ROM loaded: %s", rom_path);

    size_t save_size;
    uint8_t *save = ReadFile(SAVE_PATH, &save_size);
    if (save)
    {
        ffta_core_load_save(core, save, save_size);
        sSaveHash = Hash(save, save_size);
        free(save);
        FFTA_log("save loaded (%zu bytes)", save_size);
    }

    ffta_core_set_config(core, CONFIG_PATH);
    plat_audio_open(AUDIO_RATE);
    Settings settings;
    memset(&settings, 0, sizeof(settings));

    uint64_t next = plat_ticks_us(), last_save_check = next;
    uint32_t prev_buttons = 0, prev_stick = 0;
    bool fast_toggle = false;
    int fps = 0, counted = 0;
    uint64_t fps_since = plat_ticks_us();
    ShowToast("toast-menu"); /* only here: there is a cartridge */
    ps5_crash_step("the game");
    for (;;)
    {
        PadState pads[FFTA_MAX_PADS];
        bool quit = false;
        plat_poll(pads, &quit);
        if (quit)
            break;
        const PadState *pad = &pads[0];
        if (!pad->connected)
            for (int i = 1; i < FFTA_MAX_PADS; ++i)
                if (pads[i].connected)
                    pad = &pads[i];
        uint32_t buttons = pad->buttons;
        uint32_t stick = 0; /* the left stick as d-pad bits */
        if (pad->ly < -STICK_DEADZONE) stick |= BIT(BTN_UP);
        if (pad->ly > STICK_DEADZONE) stick |= BIT(BTN_DOWN);
        if (pad->lx < -STICK_DEADZONE) stick |= BIT(BTN_LEFT);
        if (pad->lx > STICK_DEADZONE) stick |= BIT(BTN_RIGHT);
        uint32_t pressed = (buttons | stick) & ~(prev_buttons | prev_stick);
        prev_buttons = buttons;
        prev_stick = stick;
        /* the scaling and grid of the menu: plain nearest needs no shader, the others go through the PC's pass */
        {
            int scaling = ffta_core_scaling(core);
            float grid = ffta_core_grid(core);
            bool plain = scaling == 1 && grid <= 0.001f;
            plat_game_fx(plain ? -1 : scaling, grid);
            settings.smooth = !plain;
        }

        const uint32_t select_bits = BIT(BTN_MENU) | BIT(BTN_SELECT);
        if (!ffta_core_menu_is_open(core))
        {
            /* touchpad + Options: the menu */
            if ((buttons & select_bits) && (buttons & BIT(BTN_START)) && (pressed & (select_bits | BIT(BTN_START))))
            {
                ffta_core_menu_set_open(core, 1);
                plat_audio_clear();
                continue;
            }
            if (pressed & BIT(BTN_L1))
                ffta_core_zoom(core, 1); /* LB: zoom out */
            if (pressed & BIT(BTN_R1))
                ffta_core_zoom(core, -1); /* RB: zoom in */
            if (pressed & BIT(BTN_R3))
                fast_toggle = !fast_toggle; /* right stick click: toggle fast forward */
            bool fast = fast_toggle || (buttons & BIT(BTN_L3)); /* left stick click: hold */

            int run_frames = fast ? ffta_core_ff_speed(core) : 1;
            ffta_core_run(core, GbaKeys(pad), run_frames);
            counted += run_frames;
            if (plat_ticks_us() - fps_since >= 1000000u)
            {
                fps = counted;
                counted = 0;
                fps_since += 1000000u;
            }

            size_t frames;
            const int16_t *audio = ffta_core_audio(core, &frames);
            if (frames) /* what the menu's fast-forward audio mode leaves (nothing when silent) */
            {
                /* keep about three frames of sound queued: nudge the rate to follow the clock */
                double queued = (double)plat_audio_queued_frames(), target = 3.0 * AUDIO_RATE / 60.0;
                double ratio = 1.0 + (queued - target) / target * 0.02;
                plat_audio_set_ratio(ratio < 0.98 ? 0.98 : ratio > 1.02 ? 1.02 : ratio);
                plat_audio_push(audio, frames);
            }
            else if (fast)
                plat_audio_clear();

            int w, h;
            size_t pitch;
            const uint32_t *pixels = ffta_core_video(core, &w, &h, &pitch);
            plat_begin_frame(0xFF000000);
            if (pixels)
            {
                plat_upload_game(pixels, w, h, pitch, 1, 1, 0);
                plat_draw_game(&settings, (float)w / (float)h, 255);
                DrawUiLayers(core);
                DrawToast();
                if (ffta_core_show_fps(core))
                    DrawFps(fps);
            }
            plat_end_frame();
        }
        else
        {
            /* the game waits: d-pad or left stick move, Cross accepts, Circle goes back */
            static const struct { int bit, input; } nav[] = {{BTN_UP, 0}, {BTN_DOWN, 1}, {BTN_LEFT, 2}, {BTN_RIGHT, 3},
                                                              {BTN_CROSS, 4}, {BTN_CIRCLE, 5}};
            int result = 0;
            for (size_t i = 0; i < sizeof(nav) / sizeof(nav[0]) && !result; ++i)
                if (pressed & BIT(nav[i].bit))
                    result = ffta_core_menu_input(core, nav[i].input);
            if (result == 2)
                break; /* Exit > Yes */
            if (result == 1)
                prev_buttons = buttons; /* resume: what is held now is not a press for the game */

            if (ffta_core_menu_is_open(core))
            {
                static uint32_t canvas[1280 * 720];
                int w, h;
                const uint8_t *rgba = ffta_core_menu_draw(core, &w, &h);
                for (int i = 0; i < w * h; ++i)
                    canvas[i] = 0xFF000000u | (uint32_t)rgba[i * 4] << 16 | (uint32_t)rgba[i * 4 + 1] << 8 | rgba[i * 4 + 2];
                plat_begin_frame(0xFF000000);
                plat_game_fx(-1, 0);
                settings.smooth = true;
                plat_upload_game(canvas, w, h, (size_t)w * 4, 1, 1, 0);
                plat_draw_game(&settings, (float)w / (float)h, 255);
                plat_end_frame();
            }
        }

        uint64_t now = plat_ticks_us();
        if (now - last_save_check > 5000000u)
        {
            last_save_check = now;
            WriteSave(core, false);
        }
        next += FRAME_US;
        if (now < next)
            plat_sleep_us((uint32_t)(next - now));
        else if (now - next > 100000u)
            next = now; /* far behind: do not try to catch up */
    }
    WriteSave(core, false);
    FFTA_log("quit");
    Exit(0);
    return 0;
}
