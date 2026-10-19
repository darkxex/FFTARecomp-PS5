/*
 * FFTARecomp - crash reports into FFTARecomp.log (PS5 only; no-ops elsewhere).
 * SPDX-License-Identifier: GPL-3.0-or-later
 */
#ifndef FFTA_PS5_CRASH_H
#define FFTA_PS5_CRASH_H

#if defined(__PROSPERO__)
void ps5_crash_install(const char *log_path);
/* Names the step in progress; a crash report says which one it was. */
void ps5_crash_step(const char *step);
#else
#define ps5_crash_install(path) ((void)(path))
#define ps5_crash_step(step) ((void)(step))
#endif

#endif
