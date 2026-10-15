#include <libproc.h>
#include <stdint.h>
#include <sys/resource.h>

int saturday_voice_worker_physical_footprint(pid_t pid, uint64_t *bytes_out) {
    if (pid <= 0 || bytes_out == NULL) {
        return -1;
    }
    struct rusage_info_v4 usage = {0};
    int status = proc_pid_rusage(pid, RUSAGE_INFO_V4, (rusage_info_t *)&usage);
    if (status != 0) {
        return -1;
    }
    *bytes_out = usage.ri_phys_footprint;
    return 0;
}
