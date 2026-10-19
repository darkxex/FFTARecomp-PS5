/*
 * FFTARecomp - writes a crash report into FFTARecomp.log on the PS5.
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * The Shell only says "something went wrong". On a fatal signal this logs
 * the signal, the faulting address, the instruction pointer and a short
 * frame-pointer walk, each also as an offset from main(), so the CI build's
 * symbol map (FFTARecomp-symbols.txt) turns them into function names.
 */
#if defined(__PROSPERO__)
#include "ps5_crash.h"

#include <fcntl.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <ucontext.h>
#include <unistd.h>

extern int main(void);

static int log_fd = -1;
static const char *last_step = "start";

void ps5_crash_step(const char *step)
{
    last_step = step;
}

static void put(const char *s)
{
    if (log_fd >= 0)
        (void)write(log_fd, s, strlen(s));
}

static void report_address(const char *label, uintptr_t addr)
{
    char line[160];
    intptr_t offset = (intptr_t)addr - (intptr_t)(uintptr_t)&main;
    snprintf(line, sizeof(line), "  %s 0x%llx (main%+lld)\n", label, (unsigned long long)addr,
             (long long)offset);
    put(line);
}

static void on_crash(int sig, siginfo_t *info, void *context)
{
    char line[200];
    snprintf(line, sizeof(line), "CRASH: signal %d at address %p during '%s'; main at %p\n", sig,
             info ? info->si_addr : NULL, last_step, (void *)(uintptr_t)&main);
    put(line);
    ucontext_t *uc = (ucontext_t *)context;
    if (uc)
    {
        uintptr_t rip = (uintptr_t)uc->uc_mcontext.mc_rip;
        uintptr_t rbp = (uintptr_t)uc->uc_mcontext.mc_rbp;
        uintptr_t rsp = (uintptr_t)uc->uc_mcontext.mc_rsp;
        report_address("rip", rip);
        {
            char regs[300];
            snprintf(regs, sizeof(regs), "  rsp 0x%llx rbp 0x%llx rax 0x%llx rbx 0x%llx rcx 0x%llx rdx 0x%llx rsi 0x%llx rdi 0x%llx\n",
                     (unsigned long long)rsp, (unsigned long long)rbp, (unsigned long long)uc->uc_mcontext.mc_rax,
                     (unsigned long long)uc->uc_mcontext.mc_rbx, (unsigned long long)uc->uc_mcontext.mc_rcx,
                     (unsigned long long)uc->uc_mcontext.mc_rdx, (unsigned long long)uc->uc_mcontext.mc_rsi,
                     (unsigned long long)uc->uc_mcontext.mc_rdi);
            put(regs);
        }
        {
            char more[300];
            snprintf(more, sizeof(more), "  r8 0x%llx r9 0x%llx r10 0x%llx r11 0x%llx r12 0x%llx r13 0x%llx r14 0x%llx r15 0x%llx eflags 0x%llx\n",
                     (unsigned long long)uc->uc_mcontext.mc_r8, (unsigned long long)uc->uc_mcontext.mc_r9,
                     (unsigned long long)uc->uc_mcontext.mc_r10, (unsigned long long)uc->uc_mcontext.mc_r11,
                     (unsigned long long)uc->uc_mcontext.mc_r12, (unsigned long long)uc->uc_mcontext.mc_r13,
                     (unsigned long long)uc->uc_mcontext.mc_r14, (unsigned long long)uc->uc_mcontext.mc_r15,
                     (unsigned long long)uc->uc_mcontext.mc_rflags);
            put(more);
        }
        if (rsp < ((uintptr_t)1 << 32))
        {
            /* a stack below 4 GiB is the game's: it is read only inside its mapping, and its frames
               (rbp chain, the game is built with frame pointers) are listed from there */
            const uintptr_t lo = 0x10000000u, hi = 0x1ce40000u;
            char line[100];
            if (rsp >= lo && rsp + 32 < hi)
            {
                snprintf(line, sizeof(line), "  guest stack: [rsp] 0x%llx, [rsp+8] 0x%llx\n", (unsigned long long)((uintptr_t *)rsp)[0],
                         (unsigned long long)((uintptr_t *)rsp)[1]);
                put(line);
            }
            uintptr_t frame = rbp;
            for (int depth = 0; depth < 24 && frame >= lo && frame + 16 < hi && (frame & 7) == 0; ++depth)
            {
                snprintf(line, sizeof(line), "  guest frame %d: return 0x%llx\n", depth, (unsigned long long)((uintptr_t *)frame)[1]);
                put(line);
                uintptr_t next = ((uintptr_t *)frame)[0];
                if (next <= frame)
                    break;
                frame = next;
            }
            put("END CRASH\n");
            if (log_fd >= 0)
                fsync(log_fd);
            signal(sig, SIG_DFL);
            raise(sig);
            return;
        }
        /* return addresses near the top of the stack (works without frame pointers) */
        uintptr_t *stack = (uintptr_t *)rsp;
        uintptr_t lo = (uintptr_t)&main - 0x4000000, hi = (uintptr_t)&main + 0x4000000;
        int found = 0;
        for (int i = 0; i < 512 && found < 12; ++i)
        {
            uintptr_t v = stack[i];
            if (v > lo && v < hi)
            {
                report_address("stack", v);
                ++found;
            }
        }
        (void)rbp;
    }
    put("END CRASH\n");
    if (log_fd >= 0)
        fsync(log_fd);
    signal(sig, SIG_DFL);
    raise(sig);
}

void ps5_crash_install(const char *log_path)
{
    log_fd = open(log_path, O_WRONLY | O_APPEND | O_CREAT, 0666);
    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    static char alt_stack[1 << 17];
    stack_t alt = {.ss_sp = alt_stack, .ss_size = sizeof(alt_stack), .ss_flags = 0};
    sigaltstack(&alt, NULL);
    sa.sa_sigaction = on_crash;
    sa.sa_flags = SA_SIGINFO | SA_ONSTACK;
    sigemptyset(&sa.sa_mask);
    const int signals[] = {SIGSEGV, SIGBUS, SIGILL, SIGFPE, SIGABRT, SIGSYS};
    for (size_t i = 0; i < sizeof(signals) / sizeof(signals[0]); ++i)
        sigaction(signals[i], &sa, NULL);
}
#endif
