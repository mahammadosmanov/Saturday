#include "whisper_bridge.h"

#include <whisper.h>

#include <cerrno>
#include <algorithm>
#include <array>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <limits>

#include <fcntl.h>
#include <sys/stat.h>
#include <unistd.h>

namespace {

constexpr uint32_t kMaxSamples = 480'000;
constexpr size_t kTextBytes = SATURDAY_WHISPER_MAX_TEXT_BYTES;
constexpr size_t kLanguageCodeBytes = SATURDAY_WHISPER_LANGUAGE_CODE_BYTES;
constexpr size_t kMaxLanguages = SATURDAY_WHISPER_MAX_LANGUAGES;

struct FdModelLoader {
    int fd;
    uint64_t size;
    uint64_t offset;
    bool failed;
    bool interrupted;
    SaturdayWhisperAbortCallback should_abort;
    void *abort_context;
};

bool is_aborted(FdModelLoader *loader) {
    if (loader->should_abort != nullptr && loader->should_abort(loader->abort_context) != 0) {
        loader->interrupted = true;
        loader->failed = true;
        loader->offset = loader->size;
        return true;
    }
    return false;
}

size_t read_model(void *context, void *output, size_t read_size) {
    auto *loader = static_cast<FdModelLoader *>(context);
    if (read_size == 0) {
        return 0;
    }
    if (loader->failed || is_aborted(loader) || loader->offset > loader->size ||
        static_cast<uint64_t>(read_size) > loader->size - loader->offset) {
        std::memset(output, 0, read_size);
        loader->failed = true;
        loader->offset = loader->size;
        return read_size;
    }

    auto *bytes = static_cast<uint8_t *>(output);
    size_t completed = 0;
    while (completed < read_size) {
        if (is_aborted(loader)) {
            std::memset(bytes + completed, 0, read_size - completed);
            return read_size;
        }
        const uint64_t absolute = loader->offset + completed;
        const size_t remaining = read_size - completed;
        const ssize_t amount = pread(loader->fd, bytes + completed, remaining, static_cast<off_t>(absolute));
        if (amount < 0 && errno == EINTR) {
            continue;
        }
        if (amount <= 0) {
            std::memset(bytes + completed, 0, read_size - completed);
            loader->failed = true;
            loader->offset = loader->size;
            return read_size;
        }
        completed += static_cast<size_t>(amount);
    }
    loader->offset += read_size;
    return read_size;
}

bool model_eof(void *context) {
    auto *loader = static_cast<FdModelLoader *>(context);
    return loader->failed || loader->offset >= loader->size;
}

void leave_model_fd_open(void *) {}

void discard_log(enum ggml_log_level, const char *, void *) {}

bool whisper_abort(void *context) {
    auto *loader = static_cast<FdModelLoader *>(context);
    return is_aborted(loader);
}

void set_failure(SaturdayWhisperResult *result, SaturdayWhisperStatus status) {
    result->status = static_cast<int32_t>(status);
    result->text_length = 0;
    result->evidence_count = 0;
    result->quality_flags = 0;
    result->has_no_speech_score = 0;
    result->no_speech_score = 0.0F;
    std::memset(result->text, 0, sizeof(result->text));
    std::memset(result->languages, 0, sizeof(result->languages));
}

bool valid_language_code(const char *code, size_t *length) {
    if (code == nullptr) {
        return false;
    }
    const size_t code_length = strnlen(code, kLanguageCodeBytes);
    if (code_length == 0 || code_length >= kLanguageCodeBytes) {
        return false;
    }
    for (size_t index = 0; index < code_length; ++index) {
        const char value = code[index];
        if (!((value >= 'a' && value <= 'z') || (value >= '0' && value <= '9'))) {
            return false;
        }
    }
    *length = code_length;
    return true;
}

int language_id_for_mode(uint32_t mode) {
    switch (mode) {
        case SATURDAY_WHISPER_LANGUAGE_TR:
            return whisper_lang_id("tr");
        case SATURDAY_WHISPER_LANGUAGE_EN:
            return whisper_lang_id("en");
        default:
            return -1;
    }
}

bool append_segment_text(
    whisper_context *context,
    int segment_count,
    SaturdayWhisperResult *result) {
    for (int segment = 0; segment < segment_count; ++segment) {
        const char *text = whisper_full_get_segment_text(context, segment);
        if (text == nullptr) {
            return false;
        }
        const size_t source_length = strnlen(text, kTextBytes + 1);
        const size_t available = kTextBytes - result->text_length;
        const size_t copied = std::min(source_length, available);
        if (copied > 0) {
            std::memcpy(result->text + result->text_length, text, copied);
            result->text_length += static_cast<uint32_t>(copied);
        }
        if (source_length > copied) {
            result->quality_flags |= SATURDAY_WHISPER_FLAG_OVERLONG;
            return true;
        }
    }
    return true;
}

int collect_language_evidence(
    whisper_context *context,
    int thread_count,
    SaturdayWhisperResult *result) {
    const int max_language_id = whisper_lang_max_id();
    if (max_language_id < 0 || max_language_id + 1 > static_cast<int>(kMaxLanguages)) {
        result->status = SATURDAY_WHISPER_FAILED;
        return -1;
    }

    const size_t count = static_cast<size_t>(max_language_id + 1);
    std::array<float, kMaxLanguages> probabilities{};
    const int detected = whisper_lang_auto_detect(context, 0, thread_count, probabilities.data());
    if (detected < 0 || detected > max_language_id) {
        result->status = SATURDAY_WHISPER_FAILED;
        return -1;
    }

    std::array<size_t, kMaxLanguages> order{};
    for (size_t index = 0; index < count; ++index) {
        const float score = probabilities[index];
        const char *code = whisper_lang_str(static_cast<int>(index));
        size_t code_length = 0;
        if (!std::isfinite(score) || score < 0.0F || score > 1.0F ||
            !valid_language_code(code, &code_length)) {
            result->status = SATURDAY_WHISPER_FAILED;
            return -1;
        }
        order[index] = index;
    }

    for (size_t index = 1; index < count; ++index) {
        const size_t selected = order[index];
        size_t position = index;
        while (position > 0 && probabilities[order[position - 1]] < probabilities[selected]) {
            order[position] = order[position - 1];
            --position;
        }
        order[position] = selected;
    }

    result->evidence_count = static_cast<uint32_t>(count);
    for (size_t rank = 0; rank < count; ++rank) {
        const size_t language_index = order[rank];
        const char *code = whisper_lang_str(static_cast<int>(language_index));
        size_t code_length = 0;
        if (!valid_language_code(code, &code_length)) {
            result->status = SATURDAY_WHISPER_FAILED;
            return -1;
        }
        auto &evidence = result->languages[rank];
        std::memcpy(evidence.code, code, code_length);
        evidence.code[code_length] = '\0';
        evidence.score = probabilities[language_index];
        evidence.score_gap = rank + 1 < count
            ? probabilities[language_index] - probabilities[order[rank + 1]]
            : 0.0F;
    }

    const char *best_code = whisper_lang_str(static_cast<int>(order[0]));
    if (best_code == nullptr ||
        (std::strcmp(best_code, "tr") != 0 && std::strcmp(best_code, "en") != 0)) {
        result->quality_flags |= SATURDAY_WHISPER_FLAG_UNSUPPORTED_LANGUAGE;
    }
    return static_cast<int>(order[0]);
}

void collect_explicit_language_evidence(
    uint32_t mode,
    int selected_language_id,
    whisper_context *context,
    SaturdayWhisperResult *result) {
    const int actual_language = whisper_full_lang_id(context);
    if (actual_language != selected_language_id) {
        result->quality_flags |= SATURDAY_WHISPER_FLAG_UNSUPPORTED_LANGUAGE;
    }
    if (mode != SATURDAY_WHISPER_LANGUAGE_TR && mode != SATURDAY_WHISPER_LANGUAGE_EN) {
        result->status = SATURDAY_WHISPER_INVALID_ARGUMENT;
    }
}

void collect_speech_quality(whisper_context *context, const whisper_full_params &params, SaturdayWhisperResult *result) {
    const int segment_count = whisper_full_n_segments(context);
    if (segment_count == 0) {
        result->quality_flags |= SATURDAY_WHISPER_FLAG_NO_SPEECH;
        return;
    }
    if (segment_count < 0 || segment_count > 4096) {
        result->quality_flags |= SATURDAY_WHISPER_FLAG_INCOMPLETE;
        return;
    }

    float maximum_no_speech = 0.0F;
    bool all_segments_no_speech = true;
    bool no_speech_evidence_complete = true;
    double token_log_probability = 0.0;
    uint64_t token_count = 0;
    for (int segment = 0; segment < segment_count; ++segment) {
        const float no_speech = whisper_full_get_segment_no_speech_prob(context, segment);
        if (!std::isfinite(no_speech) || no_speech < 0.0F || no_speech > 1.0F) {
            result->quality_flags |= SATURDAY_WHISPER_FLAG_INCOMPLETE;
            all_segments_no_speech = false;
            no_speech_evidence_complete = false;
            continue;
        }
        result->has_no_speech_score = 1;
        maximum_no_speech = std::max(maximum_no_speech, no_speech);
        all_segments_no_speech = all_segments_no_speech && no_speech >= params.no_speech_thold;

        const int tokens = whisper_full_n_tokens(context, segment);
        if (tokens < 0 || tokens > 8192) {
            result->quality_flags |= SATURDAY_WHISPER_FLAG_INCOMPLETE;
            continue;
        }
        for (int token = 0; token < tokens; ++token) {
            const whisper_token_data data = whisper_full_get_token_data(context, segment, token);
            if (!std::isfinite(data.plog)) {
                result->quality_flags |= SATURDAY_WHISPER_FLAG_LOW_SIGNAL;
                continue;
            }
            token_log_probability += data.plog;
            ++token_count;
        }
    }
    if (no_speech_evidence_complete && result->has_no_speech_score != 0) {
        result->no_speech_score = maximum_no_speech;
    } else {
        result->has_no_speech_score = 0;
        result->no_speech_score = 0.0F;
    }
    if (no_speech_evidence_complete && all_segments_no_speech) {
        result->quality_flags |= SATURDAY_WHISPER_FLAG_NO_SPEECH;
    }
    if (token_count == 0 || token_log_probability / static_cast<double>(token_count) < params.logprob_thold) {
        result->quality_flags |= SATURDAY_WHISPER_FLAG_LOW_SIGNAL;
    }
}

} // namespace

extern "C" int32_t saturday_whisper_transcribe_fd(
    int model_fd,
    const float *samples,
    uint32_t sample_count,
    uint32_t language_mode,
    SaturdayWhisperAbortCallback should_abort,
    void *abort_context,
    SaturdayWhisperResult *result) {
    if (result == nullptr) {
        return SATURDAY_WHISPER_INVALID_ARGUMENT;
    }
    std::memset(result, 0, sizeof(*result));
    result->status = SATURDAY_WHISPER_INVALID_ARGUMENT;
    if (model_fd < 0 || samples == nullptr || sample_count == 0 || sample_count > kMaxSamples ||
        should_abort == nullptr || language_mode > SATURDAY_WHISPER_LANGUAGE_AUTO) {
        return result->status;
    }

    for (uint32_t index = 0; index < sample_count; ++index) {
        if (!std::isfinite(samples[index]) || samples[index] < -1.0F || samples[index] > 1.0F) {
            return result->status;
        }
    }
    if (should_abort(abort_context) != 0) {
        result->status = SATURDAY_WHISPER_CANCELLED;
        return result->status;
    }

    struct stat model_stat {};
    const int access_flags = fcntl(model_fd, F_GETFL);
    if (access_flags < 0 || (access_flags & O_ACCMODE) != O_RDONLY ||
        fstat(model_fd, &model_stat) != 0 || !S_ISREG(model_stat.st_mode) || model_stat.st_size <= 0) {
        result->status = SATURDAY_WHISPER_INVALID_MODEL;
        return result->status;
    }

    ggml_log_set(discard_log, nullptr);
    FdModelLoader model_state {
        model_fd,
        static_cast<uint64_t>(model_stat.st_size),
        0,
        false,
        false,
        should_abort,
        abort_context,
    };
    whisper_model_loader loader {
        &model_state,
        read_model,
        model_eof,
        leave_model_fd_open,
    };
    whisper_context_params context_params = whisper_context_default_params();
    context_params.use_gpu = true;
    context_params.gpu_device = 0;
    whisper_context *context = whisper_init_with_params(&loader, context_params);
    if (context == nullptr || model_state.failed) {
        if (context != nullptr) {
            whisper_free(context);
        }
        result->status = model_state.interrupted || should_abort(abort_context) != 0
            ? SATURDAY_WHISPER_CANCELLED
            : SATURDAY_WHISPER_INVALID_MODEL;
        return result->status;
    }
    if (!whisper_is_multilingual(context)) {
        whisper_free(context);
        result->status = SATURDAY_WHISPER_UNSUPPORTED_MODEL;
        return result->status;
    }

    const int thread_count = 4;
    int selected_language_id = -1;
    if (language_mode == SATURDAY_WHISPER_LANGUAGE_TR || language_mode == SATURDAY_WHISPER_LANGUAGE_EN) {
        selected_language_id = language_id_for_mode(language_mode);
        if (selected_language_id < 0) {
            whisper_free(context);
            result->status = SATURDAY_WHISPER_UNSUPPORTED_MODEL;
            return result->status;
        }
    } else {
        if (whisper_pcm_to_mel(context, samples, static_cast<int>(sample_count), thread_count) != 0) {
            whisper_free(context);
            result->status = should_abort(abort_context) != 0
                ? SATURDAY_WHISPER_CANCELLED
                : SATURDAY_WHISPER_FAILED;
            return result->status;
        }
        selected_language_id = collect_language_evidence(context, thread_count, result);
        if (selected_language_id < 0) {
            whisper_free(context);
            return result->status;
        }
    }

    whisper_full_params params = whisper_full_default_params(WHISPER_SAMPLING_GREEDY);
    params.n_threads = thread_count;
    params.translate = false;
    params.no_context = true;
    params.no_timestamps = true;
    params.print_special = false;
    params.print_progress = false;
    params.print_realtime = false;
    params.print_timestamps = false;
    params.language = language_mode == SATURDAY_WHISPER_LANGUAGE_TR ? "tr"
        : language_mode == SATURDAY_WHISPER_LANGUAGE_EN ? "en"
        : whisper_lang_str(selected_language_id);
    params.detect_language = false;
    if (params.language == nullptr) {
        whisper_free(context);
        result->status = SATURDAY_WHISPER_FAILED;
        return result->status;
    }
    params.abort_callback = whisper_abort;
    params.abort_callback_user_data = &model_state;

    const int transcription_status = whisper_full(context, params, samples, static_cast<int>(sample_count));
    if (transcription_status != 0) {
        whisper_free(context);
        result->status = model_state.interrupted || should_abort(abort_context) != 0
            ? SATURDAY_WHISPER_CANCELLED
            : SATURDAY_WHISPER_FAILED;
        return result->status;
    }

    const int segment_count = whisper_full_n_segments(context);
    if (segment_count < 0 || segment_count > 4096 || !append_segment_text(context, segment_count, result)) {
        result->quality_flags |= SATURDAY_WHISPER_FLAG_INCOMPLETE;
    }
    if (language_mode == SATURDAY_WHISPER_LANGUAGE_AUTO) {
        if (whisper_full_lang_id(context) != selected_language_id) {
            result->quality_flags |= SATURDAY_WHISPER_FLAG_MIXED_LANGUAGE;
        }
    } else {
        collect_explicit_language_evidence(language_mode, selected_language_id, context, result);
    }
    collect_speech_quality(context, params, result);
    whisper_free(context);

    result->status = SATURDAY_WHISPER_OK;
    return result->status;
}
