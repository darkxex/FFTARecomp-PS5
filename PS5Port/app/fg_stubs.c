/* The presenter links against the frame interpolator; this build has none. */
#include <stdio.h>
#include <string.h>

#include "platform/vk/fg_bridge.h"
#include "platform/vk/vk_present.h"


/* ---- frame interpolation is not used: the presenter links against these ---- */

FgInterpolator *ssfg_create(PFN_vkVoidFunction (*get)(VkInstance, const char *), VkInstance i, VkPhysicalDevice g, VkDevice d,
                            uint32_t w, uint32_t h, VkFormat f, char *error, size_t error_size)
{
    (void)get, (void)i, (void)g, (void)d, (void)w, (void)h, (void)f;
    snprintf(error, error_size, "not in this build");
    return NULL;
}
void ssfg_destroy(FgInterpolator *fg) { (void)fg; }
void ssfg_prepare(FgInterpolator *fg, VkCommandBuffer cmd) { (void)fg, (void)cmd; }
VkImage ssfg_frame_image(FgInterpolator *fg) { (void)fg; return VK_NULL_HANDLE; }
VkImageView ssfg_frame_view(FgInterpolator *fg) { (void)fg; return VK_NULL_HANDLE; }
bool ssfg_record(FgInterpolator *fg, VkCommandBuffer cmd, bool reset) { (void)fg, (void)cmd, (void)reset; return false; }
VkImageView ssfg_output_view(FgInterpolator *fg) { (void)fg; return VK_NULL_HANDLE; }
bool ssfg_record_n(FgInterpolator *fg, VkCommandBuffer cmd, bool reset, uint32_t n) { (void)fg, (void)cmd, (void)reset, (void)n; return false; }
VkImageView ssfg_generated_view(FgInterpolator *fg, uint32_t i) { (void)fg, (void)i; return VK_NULL_HANDLE; }
void ssfg_advance(FgInterpolator *fg) { (void)fg; }
FgPacing *ssfg_pacing_create(void) { return NULL; }
void ssfg_pacing_destroy(FgPacing *p) { (void)p; }
FgDecision ssfg_pacing_frame(FgPacing *p, uint32_t a, uint32_t b, double c, double d, double e, double f, bool g, uint32_t h)
{
    (void)p, (void)a, (void)b, (void)c, (void)d, (void)e, (void)f, (void)g, (void)h;
    FgDecision none;
    memset(&none, 0, sizeof(none));
    return none;
}
const char *ssfg_pacing_state_name(int s) { (void)s; return ""; }
const char *ssfg_pacing_event(FgPacing *p) { (void)p; return NULL; }

/* The link wraps fclose and fflush (the platform layer's memory streams); the originals are kept under these names. */
int ps5_memstream_fclose(FILE *f);
int ps5_memstream_fflush(FILE *f);
int __wrap_fclose(FILE *f) { return ps5_memstream_fclose(f); }
int __wrap_fflush(FILE *f) { return ps5_memstream_fflush(f); }
