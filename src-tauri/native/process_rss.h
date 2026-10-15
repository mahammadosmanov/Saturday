#ifndef SATURDAY_PROCESS_RSS_H
#define SATURDAY_PROCESS_RSS_H

#include <stdint.h>
#include <sys/types.h>

int saturday_voice_worker_task_identity(pid_t pid, uint64_t *identity_out);
int saturday_voice_worker_physical_footprint(pid_t pid, uint64_t expected_identity, uint64_t *bytes_out);
int saturday_voice_worker_signal(pid_t pid, uint64_t expected_identity, int signal_number);

#endif
