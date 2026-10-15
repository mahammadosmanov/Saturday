#include "llama_adapter.h"
#include "llama.h"

#include <algorithm>
#include <array>
#include <atomic>
#include <cerrno>
#include <climits>
#include <cstdio>
#include <cstring>
#include <fcntl.h>
#include <mutex>
#include <new>
#include <string>
#include <sys/stat.h>
#include <unistd.h>
namespace {
constexpr uint32_t kContextTokens = 4'096;
constexpr uint32_t kPromptTokens = 3'840;
constexpr uint32_t kOutputTokens = 256;
constexpr size_t kPromptBytes = 128 * 1024;
constexpr size_t kOutputBytes = 8'192;
constexpr size_t kDeltaBytes = 4'096;
constexpr size_t kTokenPieceBytes = 4'096;
std::once_flag backend_init_once;

/* Return the maximal well-formed UTF-8 prefix. Incomplete final scalars wait
 * for the next sampled token; malformed token bytes fail closed. */
int utf8_prefix(const std::string &s, size_t &prefix) {
    size_t i = 0;
    while (i < s.size()) {
        const auto lead = static_cast<unsigned char>(s[i]);
        size_t width = 0;
        uint32_t scalar = 0;
        if (lead < 0x80) {
            width = 1;
            scalar = lead;
        } else if ((lead & 0xE0) == 0xC0) {
            width = 2;
            scalar = lead & 0x1F;
        } else if ((lead & 0xF0) == 0xE0) {
            width = 3;
            scalar = lead & 0x0F;
        } else if ((lead & 0xF8) == 0xF0) {
            width = 4;
            scalar = lead & 0x07;
        } else {
            return -1;
        }
        if (s.size() - i < width) {
            prefix = i;
            return 0;
        }
        for (size_t j = 1; j < width; ++j) {
            const auto byte = static_cast<unsigned char>(s[i + j]);
            if ((byte & 0xC0) != 0x80) return -1;
            scalar = (scalar << 6) | (byte & 0x3F);
        }
        if ((width == 2 && scalar < 0x80) || (width == 3 && scalar < 0x800) ||
            (width == 4 && scalar < 0x10000) || scalar > 0x10FFFF ||
            (scalar >= 0xD800 && scalar <= 0xDFFF)) return -1;
        i += width;
    }
    prefix = i;
    return 1;
}
} // namespace

struct saturday_llama {
    FILE *file = nullptr;
    llama_model *model = nullptr;
    llama_context *context = nullptr;
    llama_sampler *sampler = nullptr;
    const llama_vocab *vocab = nullptr;
    saturday_llama_abort_callback abort_callback = nullptr;
    void *abort_context = nullptr;
    std::string pending_utf8;
    size_t published_bytes = 0;
    uint32_t pending_tokens = 0;
    uint32_t input_tokens = 0;
    uint32_t output_tokens = 0;
    uint32_t maximum_output_tokens = 0;
    llama_token last_sampled_token = LLAMA_TOKEN_NULL;
    bool decode_pending = false;
    bool active = false;
    bool cancelled = false;
};

namespace {
bool abort_requested(const saturday_llama *instance) {
    return instance == nullptr || instance->abort_callback == nullptr ||
        instance->abort_callback(instance->abort_context);
}
} // namespace

struct LoadAbortContext {
    saturday_llama_abort_callback callback;
    void *context;
};

bool llama_load_progress(float, void *opaque) {
    if (opaque == nullptr) return false;
    const auto *context = static_cast<const LoadAbortContext *>(opaque);
    return !context->callback(context->context);
}
extern "C" int saturday_llama_load(int model_fd, uint64_t expected_size,
                                    saturday_llama_abort_callback abort_callback,
                                    void *abort_context, saturday_llama **out) {
    if (out == nullptr) return SATURDAY_LLAMA_INVALID;
    *out = nullptr;
    if (model_fd < 0 || expected_size == 0 || abort_callback == nullptr || abort_context == nullptr) {
        return SATURDAY_LLAMA_INVALID;
    }
    if (abort_callback(abort_context)) return SATURDAY_LLAMA_CANCELLED;

    const int flags = fcntl(model_fd, F_GETFL);
    struct stat info {};
    if (flags < 0 || (flags & O_ACCMODE) != O_RDONLY || fstat(model_fd, &info) != 0 ||
        !S_ISREG(info.st_mode) || info.st_size < 0 ||
        static_cast<uint64_t>(info.st_size) != expected_size) {
        return SATURDAY_LLAMA_INVALID;
    }

    const int duplicate = dup(model_fd);
    if (duplicate < 0) return SATURDAY_LLAMA_IO;
    FILE *file = fdopen(duplicate, "rb");
    if (file == nullptr) {
        close(duplicate);
        return SATURDAY_LLAMA_IO;
    }
    if (fseeko(file, 0, SEEK_SET) != 0) {
        fclose(file);
        return SATURDAY_LLAMA_IO;
    }

    std::call_once(backend_init_once, [] { llama_backend_init(); });
    auto *instance = new (std::nothrow) saturday_llama();
    if (instance == nullptr) {
        fclose(file);
        return SATURDAY_LLAMA_IO;
    }
    instance->file = file;
    instance->abort_callback = abort_callback;
    instance->abort_context = abort_context;

    llama_model_params model_params = llama_model_default_params();
    model_params.n_gpu_layers = 99;
    model_params.split_mode = LLAMA_SPLIT_MODE_NONE;
    model_params.load_mode = LLAMA_LOAD_MODE_MMAP;
    model_params.lazy_mode = LLAMA_LAZY_MODE_OFF;
    const LoadAbortContext load_abort {abort_callback, abort_context};
    model_params.progress_callback = llama_load_progress;
    model_params.progress_callback_user_data = const_cast<LoadAbortContext *>(&load_abort);
    instance->model = llama_model_load_from_file_ptr(file, model_params);
    if (instance->model == nullptr) {
        saturday_llama_free(instance);
        return SATURDAY_LLAMA_MODEL;
    }
    if (abort_requested(instance)) {
        saturday_llama_free(instance);
        return SATURDAY_LLAMA_CANCELLED;
    }
    instance->vocab = llama_model_get_vocab(instance->model);
    if (instance->vocab == nullptr || !llama_model_has_decoder(instance->model)) {
        saturday_llama_free(instance);
        return SATURDAY_LLAMA_MODEL;
    }
    llama_context_params context_params = llama_context_default_params();
    context_params.n_ctx = kContextTokens;
    context_params.n_batch = kContextTokens;
    context_params.n_ubatch = 512;
    context_params.n_seq_max = 1;
    context_params.n_threads = 4;
    context_params.n_threads_batch = 4;
    context_params.abort_callback = abort_callback;
    context_params.abort_callback_data = abort_context;
    instance->context = llama_init_from_model(instance->model, context_params);
    if (instance->context == nullptr || llama_n_ctx(instance->context) != kContextTokens) {
        const int failure = abort_requested(instance) ? SATURDAY_LLAMA_CANCELLED : SATURDAY_LLAMA_RUNTIME;
        saturday_llama_free(instance);
        return failure;
    }
    *out = instance;
    return SATURDAY_LLAMA_OK;
}

extern "C" void saturday_llama_free(saturday_llama *instance) {
    if (instance == nullptr) return;
    if (instance->sampler != nullptr) llama_sampler_free(instance->sampler);
    if (instance->context != nullptr) llama_free(instance->context);
    if (instance->model != nullptr) llama_model_free(instance->model);
    if (instance->file != nullptr) fclose(instance->file);
    delete instance;
}

extern "C" int32_t saturday_llama_chat_template(const saturday_llama *instance, char *out, size_t capacity) {
    if (instance == nullptr || instance->model == nullptr) return SATURDAY_LLAMA_INVALID;
    const char *value = llama_model_chat_template(instance->model, nullptr);
    if (value == nullptr) return SATURDAY_LLAMA_MODEL;
    const size_t size = std::strlen(value);
    if (size > static_cast<size_t>(INT32_MAX)) return SATURDAY_LLAMA_MODEL;
    if (out == nullptr || capacity <= size) return static_cast<int32_t>(size);
    std::memcpy(out, value, size);
    out[size] = '\0';
    return static_cast<int32_t>(size);
}

static int tokenize_prompt(const saturday_llama *instance, const saturday_llama_piece *pieces,
                           size_t piece_count, std::array<llama_token, kContextTokens> &tokens,
                           uint32_t &token_count) {
    if (instance == nullptr || pieces == nullptr || piece_count == 0 || piece_count > 128) {
        return SATURDAY_LLAMA_INVALID;
    }
    std::array<llama_token, kContextTokens> scratch {};
    size_t total_bytes = 0;
    token_count = 0;
    for (size_t i = 0; i < piece_count; ++i) {
        const auto &piece = pieces[i];
        if ((piece.bytes != 0 && piece.data == nullptr) ||
            (piece.parse_special_tokens != 0 && piece.parse_special_tokens != 1) ||
            piece.bytes > kPromptBytes || total_bytes > kPromptBytes - piece.bytes) {
            return SATURDAY_LLAMA_INVALID;
        }
        total_bytes += piece.bytes;
        if (piece.bytes == 0) continue;
        const int32_t count = llama_tokenize(instance->vocab, piece.data,
            static_cast<int32_t>(piece.bytes), scratch.data(), static_cast<int32_t>(scratch.size()),
            false, piece.parse_special_tokens != 0);
        if (count < 0 || static_cast<uint32_t>(count) > kPromptTokens - token_count) {
            return SATURDAY_LLAMA_INVALID;
        }
        std::copy_n(scratch.begin(), count, tokens.begin() + token_count);
        token_count += static_cast<uint32_t>(count);
    }
    return token_count == 0 ? SATURDAY_LLAMA_INVALID : SATURDAY_LLAMA_OK;
}

extern "C" int saturday_llama_count_prompt_tokens(const saturday_llama *instance,
    const saturday_llama_piece *pieces, size_t piece_count, uint32_t *input_tokens) {
    if (input_tokens == nullptr) return SATURDAY_LLAMA_INVALID;
    std::array<llama_token, kContextTokens> tokens {};
    uint32_t count = 0;
    const int result = tokenize_prompt(instance, pieces, piece_count, tokens, count);
    if (result == SATURDAY_LLAMA_OK) *input_tokens = count;
    return result;
}

extern "C" int saturday_llama_start(saturday_llama *instance,
    const saturday_llama_piece *pieces, size_t piece_count,
    uint32_t maximum_output_tokens, uint32_t *input_tokens) {
    if (instance == nullptr || input_tokens == nullptr || instance->active ||
        maximum_output_tokens == 0 || maximum_output_tokens > kOutputTokens) {
        return SATURDAY_LLAMA_INVALID;
    }
    if (abort_requested(instance)) return SATURDAY_LLAMA_CANCELLED;
    if (instance->sampler != nullptr) {
        llama_sampler_free(instance->sampler);
        instance->sampler = nullptr;
    }
    llama_memory_clear(llama_get_memory(instance->context), true);

    std::array<llama_token, kContextTokens> tokens {};
    uint32_t count = 0;
    const int tokenized = tokenize_prompt(instance, pieces, piece_count, tokens, count);
    if (tokenized != SATURDAY_LLAMA_OK) return tokenized;
    const int32_t decoded = llama_decode(instance->context,
        llama_batch_get_one(tokens.data(), static_cast<int32_t>(count)));
    if (decoded != 0) return abort_requested(instance) ? SATURDAY_LLAMA_CANCELLED : SATURDAY_LLAMA_RUNTIME;
    if (abort_requested(instance)) return SATURDAY_LLAMA_CANCELLED;

    auto sampler_params = llama_sampler_chain_default_params();
    instance->sampler = llama_sampler_chain_init(sampler_params);
    if (instance->sampler == nullptr) return SATURDAY_LLAMA_RUNTIME;
    llama_sampler_chain_add(instance->sampler, llama_sampler_init_top_k(20));
    llama_sampler_chain_add(instance->sampler, llama_sampler_init_top_p(0.8f, 1));
    llama_sampler_chain_add(instance->sampler, llama_sampler_init_temp(0.7f));
    llama_sampler_chain_add(instance->sampler, llama_sampler_init_dist(LLAMA_DEFAULT_SEED));

    instance->pending_utf8.clear();
    instance->published_bytes = 0;
    instance->pending_tokens = 0;
    instance->input_tokens = count;
    instance->output_tokens = 0;
    instance->maximum_output_tokens = maximum_output_tokens;
    instance->cancelled = false;
    instance->decode_pending = false;
    instance->last_sampled_token = LLAMA_TOKEN_NULL;
    instance->active = true;
    *input_tokens = instance->input_tokens;
    return SATURDAY_LLAMA_OK;
}

extern "C" int saturday_llama_poll(saturday_llama *instance, size_t max_delta_bytes,
                                    char *delta, size_t delta_capacity, size_t *delta_bytes,
                                    uint32_t *delta_tokens, uint32_t *input_tokens,
                                    uint32_t *output_tokens) {
    if (instance == nullptr || delta == nullptr || delta_bytes == nullptr || delta_tokens == nullptr ||
        input_tokens == nullptr || output_tokens == nullptr || max_delta_bytes > kDeltaBytes ||
        delta_capacity < max_delta_bytes) return SATURDAY_LLAMA_INVALID;
    *delta_bytes = 0;
    *delta_tokens = 0;
    *input_tokens = instance->input_tokens;
    *output_tokens = instance->output_tokens;
    if (instance->cancelled) return SATURDAY_LLAMA_CANCELLED;
    if (abort_requested(instance)) return saturday_llama_cancel(instance);
    if (!instance->active) return SATURDAY_LLAMA_INVALID;
    if (max_delta_bytes == 0) return SATURDAY_LLAMA_PENDING;

    if (!instance->pending_utf8.empty()) {
        size_t valid = 0;
        const int utf8 = utf8_prefix(instance->pending_utf8, valid);
        if (utf8 < 0) return SATURDAY_LLAMA_RUNTIME;
        if (utf8 == 1) {
            if (instance->pending_utf8.size() > max_delta_bytes) return SATURDAY_LLAMA_NEED_CREDIT;
            if (instance->published_bytes + instance->pending_utf8.size() > kOutputBytes ||
                instance->pending_tokens == 0) {
                return SATURDAY_LLAMA_OUTPUT_LIMIT;
            }
            const size_t bytes = instance->pending_utf8.size();
            std::memcpy(delta, instance->pending_utf8.data(), bytes);
            instance->pending_utf8.clear();
            *delta_bytes = bytes;
            *delta_tokens = instance->pending_tokens;
            instance->pending_tokens = 0;
            instance->published_bytes += bytes;
            *output_tokens = instance->output_tokens;
            return SATURDAY_LLAMA_OK;
        }
        if (instance->pending_utf8.size() > kDeltaBytes) return SATURDAY_LLAMA_OUTPUT_LIMIT;
    }

    if (instance->output_tokens >= instance->maximum_output_tokens) {
        instance->active = false;
        return SATURDAY_LLAMA_OUTPUT_LIMIT;
    }
    if (instance->decode_pending) {
        const int32_t decoded = llama_decode(instance->context,
            llama_batch_get_one(&instance->last_sampled_token, 1));
        if (decoded != 0) {
            return abort_requested(instance)
                ? saturday_llama_cancel(instance)
                : SATURDAY_LLAMA_RUNTIME;
        }
        if (abort_requested(instance)) return saturday_llama_cancel(instance);
        instance->decode_pending = false;
    }
    if (abort_requested(instance)) return saturday_llama_cancel(instance);
    const llama_token token = llama_sampler_sample(instance->sampler, instance->context, -1);
    if (llama_vocab_is_eog(instance->vocab, token)) {
        if (!instance->pending_utf8.empty()) return SATURDAY_LLAMA_RUNTIME;
        instance->active = false;
        *input_tokens = instance->input_tokens;
        *output_tokens = instance->output_tokens;
        return SATURDAY_LLAMA_COMPLETE;
    }
    if (llama_vocab_is_control(instance->vocab, token)) return SATURDAY_LLAMA_CONTROL_TOKEN;
    instance->last_sampled_token = token;
    instance->decode_pending = true;

    std::array<char, kTokenPieceBytes> piece {};
    const int32_t piece_bytes = llama_token_to_piece(instance->vocab, token, piece.data(),
                                                     static_cast<int32_t>(piece.size()), 0, false);
    if (piece_bytes <= 0 || static_cast<size_t>(piece_bytes) > piece.size()) return SATURDAY_LLAMA_RUNTIME;
    if (instance->pending_utf8.size() + static_cast<size_t>(piece_bytes) > kDeltaBytes ||
        instance->published_bytes + instance->pending_utf8.size() + static_cast<size_t>(piece_bytes) > kOutputBytes) {
        return SATURDAY_LLAMA_OUTPUT_LIMIT;
    }
    instance->pending_utf8.append(piece.data(), static_cast<size_t>(piece_bytes));
    ++instance->pending_tokens;
    ++instance->output_tokens;
    *output_tokens = instance->output_tokens;

    size_t valid = 0;
    const int utf8 = utf8_prefix(instance->pending_utf8, valid);
    if (utf8 < 0) return SATURDAY_LLAMA_RUNTIME;
    if (utf8 != 1) return SATURDAY_LLAMA_PENDING;
    if (instance->pending_utf8.size() > max_delta_bytes) return SATURDAY_LLAMA_NEED_CREDIT;
    const size_t bytes = instance->pending_utf8.size();
    std::memcpy(delta, instance->pending_utf8.data(), bytes);
    instance->pending_utf8.clear();
    *delta_bytes = bytes;
    *delta_tokens = instance->pending_tokens;
    instance->pending_tokens = 0;
    instance->published_bytes += bytes;
    return SATURDAY_LLAMA_OK;
}

extern "C" int saturday_llama_cancel(saturday_llama *instance) {
    if (instance == nullptr) return SATURDAY_LLAMA_INVALID;
    instance->cancelled = true;
    instance->active = false;
    instance->pending_utf8.clear();
    instance->pending_tokens = 0;
    instance->decode_pending = false;
    llama_memory_clear(llama_get_memory(instance->context), true);
    return SATURDAY_LLAMA_CANCELLED;
}
