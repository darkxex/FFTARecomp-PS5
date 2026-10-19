/*
 * FFTARecomp - asking the HEN to let FFTARecomp list /data.
 * SPDX-License-Identifier: GPL-3.0-or-later
 */
#ifndef FFTA_PS5_UNLOCK_H
#define FFTA_PS5_UNLOCK_H

#include <stdbool.h>

typedef enum
{
    UNLOCK_OK,
    UNLOCK_ALREADY,
    UNLOCK_NO_ANSWER,
    UNLOCK_CANT_REQUEST,
    UNLOCK_SKIPPED_AFTER_CRASH,
} UnlockResult;

bool ps5_data_listable(void);
/* Call first thing in main(), before any thread exists. */
UnlockResult ps5_unlock_etahen(void);
/* One short request by file (~0.3 s), for when another way is known to work: a service that is loaded answers at once. */
UnlockResult ps5_unlock_quick(void);
void ps5_unlock_reset_crash_marker(void);
const char *ps5_unlock_describe(UnlockResult result);

#endif
