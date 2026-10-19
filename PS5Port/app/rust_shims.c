/* Symbols the Rust standard library expects that the console's libc lacks. */
struct dl_phdr_info;

/* Only used to find unwind tables for backtraces; the core aborts on a panic. */
int dl_iterate_phdr(int (*callback)(struct dl_phdr_info *, unsigned long, void *), void *data)
{
    (void)callback, (void)data;
    return 0;
}
