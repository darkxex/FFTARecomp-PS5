/*
 * FFTARecomp - C entry point for the boilerplate's Lapy elevation client.
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * ShadowMountPlus lets a sandboxed title open files in /data but not list
 * folders, and FFTARecomp must list /data/FFTARecomp to build its library.
 */
#if defined(__PROSPERO__)
#include "elevation/elevation.hpp"

extern "C" int FFTA_elevate(const char **route)
{
    const auto status = elevation::request(elevation::Capability::filesystem);
    if (route)
        *route = elevation::path();
    return static_cast<int>(status);
}
#endif
