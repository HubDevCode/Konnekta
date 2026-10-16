/* Konnekta core C API — see crates/konnekta-ffi/src/lib.rs */
#ifndef KONNEKTA_H
#define KONNEKTA_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct KonnektaHandle KonnektaHandle;

/* Frame kinds for streams */
#define KONNEKTA_FRAME_AUDIO   1
#define KONNEKTA_FRAME_VIDEO   2
#define KONNEKTA_FRAME_CONTROL 3
/* Frame flags */
#define KONNEKTA_FLAG_KEYFRAME 1
#define KONNEKTA_FLAG_CONFIG   2

KonnektaHandle *konnekta_start(const char *config_json, const char *data_dir);
char *konnekta_call(KonnektaHandle *h, const char *method, const char *params_json);
char *konnekta_next_event(KonnektaHandle *h, int32_t timeout_ms);
int32_t konnekta_stream_write(KonnektaHandle *h, uint64_t channel, uint8_t kind, uint8_t flags,
                              uint64_t pts_us, const uint8_t *data, size_t len);
int64_t konnekta_stream_read(KonnektaHandle *h, uint64_t channel, uint8_t *buf, size_t cap,
                             uint8_t *meta, uint64_t *pts_us, int32_t timeout_ms);
void konnekta_free_string(char *s);
void konnekta_stop(KonnektaHandle *h);

#ifdef __cplusplus
}
#endif
#endif
