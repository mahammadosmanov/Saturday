#include "process_rss.h"

#include <libproc.h>
#include <sys/proc_info.h>
#include <signal.h>
#include <stdint.h>
#include <sys/resource.h>
#include <sys/types.h>
#include <unistd.h>

static int saturday_voice_worker_process_identity(pid_t pid, uint64_t *identity_out) {
    if (pid <= 0 || identity_out == NULL) {
        return -1;
    }
    struct proc_bsdinfo info = {0};
    int size = proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, sizeof(info));
    if (size != (int)sizeof(info) || info.pbi_pid != (uint32_t)pid
        || info.pbi_start_tvsec == 0 || info.pbi_start_tvusec >= 1000000) {
        return -1;
    }
    uint64_t seconds = (uint64_t)info.pbi_start_tvsec;
    uint64_t microseconds = (uint64_t)info.pbi_start_tvusec;
    if (seconds > (UINT64_MAX - microseconds) / UINT64_C(1000000)) {
        return -1;
    }
    *identity_out = seconds * UINT64_C(1000000) + microseconds;
    return *identity_out == 0 ? -1 : 0;
}

static int saturday_voice_worker_identity_matches(pid_t pid, uint64_t expected) {
    uint64_t actual = 0;
    return expected != 0 && saturday_voice_worker_process_identity(pid, &actual) == 0
        && actual == expected;
}

int saturday_voice_worker_task_identity(pid_t pid, uint64_t *identity_out) {
    return saturday_voice_worker_process_identity(pid, identity_out);
}

int saturday_voice_worker_physical_footprint(pid_t pid, uint64_t expected_identity, uint64_t *bytes_out) {
    if (bytes_out == NULL || !saturday_voice_worker_identity_matches(pid, expected_identity)) {
        return -1;
    }
    struct rusage_info_v4 usage = {0};
    if (proc_pid_rusage(pid, RUSAGE_INFO_V4, (rusage_info_t *)&usage) != 0
        || !saturday_voice_worker_identity_matches(pid, expected_identity)) {
        return -1;
    }
    *bytes_out = usage.ri_phys_footprint;
    return 0;
}

int saturday_voice_worker_signal(pid_t pid, uint64_t expected_identity, int signal_number) {
    if ((signal_number != SIGTERM && signal_number != SIGKILL)
        || !saturday_voice_worker_identity_matches(pid, expected_identity)) {
        return -1;
    }
    return kill(pid, signal_number);
}
