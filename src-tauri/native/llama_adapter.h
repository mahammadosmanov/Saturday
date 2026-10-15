#ifndef SATURDAY_LLAMA_ADAPTER_H
#define SATURDAY_LLAMA_ADAPTER_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct saturday_llama saturday_llama;
enum saturday_llama_result {
    SATURDAY_LLAMA_OK = 0,
    SATURDAY_LLAMA_PENDING = 1,
    SATURDAY_LLAMA_COMPLETE = 2,
    SATURDAY_LLAMA_CANCELLED = 3,
    SATURDAY_LLAMA_NEED_CREDIT = 4,
    SATURDAY_LLAMA_INVALID = -1,
    SATURDAY_LLAMA_IO = -2,
    SATURDAY_LLAMA_MODEL = -3,
    SATURDAY_LLAMA_RUNTIME = -4,
    SATURDAY_LLAMA_CONTROL_TOKEN = -5,
    SATURDAY_LLAMA_OUTPUT_LIMIT = -6,
};

typedef struct saturday_llama_piece {
    const char *data;
    size_t bytes;
    int parse_special_tokens;
} saturday_llama_piece;

typedef bool (*saturday_llama_abort_callback)(void *context);

/* The fd must be an approved, read-only regular GGUF file. The adapter duplicates
 * it, wraps the duplicate in FILE*, and holds that stream through llama_model_free.
 * The caller remains responsible for independently validating the authorized file
 * identity before calling this function. The abort callback's context must remain
 * alive through saturday_llama_free. */
int saturday_llama_load(int model_fd, uint64_t expected_size,
                        saturday_llama_abort_callback abort_callback,
                        void *abort_context, saturday_llama **out);
void saturday_llama_free(saturday_llama *model);

/* Copy the exact embedded tokenizer.chat_template returned by llama.cpp. Returns
 * required bytes (excluding NUL), or a negative result code. */
int32_t saturday_llama_chat_template(const saturday_llama *model, char *out, size_t capacity);

/* Prompt pieces distinguish trusted template control tokens from untrusted
 * message text, which is always tokenized with special parsing disabled. */
int saturday_llama_count_prompt_tokens(const saturday_llama *model,
    const saturday_llama_piece *pieces, size_t piece_count, uint32_t *input_tokens);
int saturday_llama_start(saturday_llama *model, const saturday_llama_piece *pieces,
    size_t piece_count, uint32_t maximum_output_tokens, uint32_t *input_tokens);

/* Advances by at most one sampled token. A positive delta is UTF-8 and never
 * exceeds max_delta_bytes. With zero credit this function does not generate. */
int saturday_llama_poll(saturday_llama *model, size_t max_delta_bytes,
                        char *delta, size_t delta_capacity, size_t *delta_bytes,
                        uint32_t *delta_tokens, uint32_t *input_tokens,
                        uint32_t *output_tokens);
int saturday_llama_cancel(saturday_llama *model);

#ifdef __cplusplus
}
#endif
#endif
