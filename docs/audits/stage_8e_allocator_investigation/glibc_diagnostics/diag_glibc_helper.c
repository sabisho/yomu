#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <malloc.h>
#include <signal.h>
#include <unistd.h>
#include <time.h>

static void dump_mallinfo2(const char* label, const char* filepath, int append) {
    FILE* fp = fopen(filepath, append ? "a" : "w");
    if (!fp) return;
    struct mallinfo2 mi = mallinfo2();
    fprintf(fp, "=== %s (timestamp: %ld) ===\n", label, (long)time(NULL));
    fprintf(fp, "arena:    %zu bytes (%zu MiB) [non-mmap space allocated from system]\n", (size_t)mi.arena, (size_t)mi.arena / (1024*1024));
    fprintf(fp, "ordblks:  %zu [number of free chunks]\n", (size_t)mi.ordblks);
    fprintf(fp, "smblks:   %zu [number of fastbin blocks]\n", (size_t)mi.smblks);
    fprintf(fp, "hblks:    %zu [number of mmapped regions]\n", (size_t)mi.hblks);
    fprintf(fp, "hblkhd:   %zu bytes (%zu MiB) [space in mmapped regions]\n", (size_t)mi.hblkhd, (size_t)mi.hblkhd / (1024*1024));
    fprintf(fp, "usmblks:  %zu\n", (size_t)mi.usmblks);
    fprintf(fp, "fsmblks:  %zu\n", (size_t)mi.fsmblks);
    fprintf(fp, "uordblks: %zu bytes (%zu MiB) [total allocated/in-use space]\n", (size_t)mi.uordblks, (size_t)mi.uordblks / (1024*1024));
    fprintf(fp, "fordblks: %zu bytes (%zu MiB) [total free space]\n", (size_t)mi.fordblks, (size_t)mi.fordblks / (1024*1024));
    fprintf(fp, "keepcost: %zu bytes (%zu MiB) [top-most, releasable space]\n\n", (size_t)mi.keepcost, (size_t)mi.keepcost / (1024*1024));
    fclose(fp);
}

static void sigusr1_handler(int sig) {
    (void)sig;
    const char* mallinfo_path = "/tmp/yomu_glibc_mallinfo.txt";
    const char* malloc_info_pre = "/tmp/yomu_glibc_malloc_info.xml";
    const char* malloc_info_post = "/tmp/yomu_glibc_malloc_info_post_trim.xml";
    const char* trim_result_path = "/tmp/yomu_glibc_trim_result.txt";

    dump_mallinfo2("BEFORE malloc_trim(0)", mallinfo_path, 0);

    FILE* fp_pre = fopen(malloc_info_pre, "w");
    if (fp_pre) {
        malloc_info(0, fp_pre);
        fclose(fp_pre);
    }

    int ret = malloc_trim(0);

    FILE* fp_trim = fopen(trim_result_path, "w");
    if (fp_trim) {
        fprintf(fp_trim, "malloc_trim(0) returned: %d\n", ret);
        fclose(fp_trim);
    }

    dump_mallinfo2("AFTER malloc_trim(0)", mallinfo_path, 1);

    FILE* fp_post = fopen(malloc_info_post, "w");
    if (fp_post) {
        malloc_info(0, fp_post);
        fclose(fp_post);
    }
}

__attribute__((constructor))
static void init_diag_glibc_helper(void) {
    struct sigaction sa;
    sa.sa_handler = sigusr1_handler;
    sigemptyset(&sa.sa_mask);
    sa.sa_flags = SA_RESTART;
    sigaction(SIGUSR1, &sa, NULL);
}
