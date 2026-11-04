#include <stdint.h>
#include <stddef.h>
typedef struct {
    int32_t status;
    uint32_t width;
    uint32_t height;
    uint32_t reserved;
    uint64_t source;
    uint8_t *rgba;
    size_t length;
    uint64_t window_identity;
} HexTeamsSnapshot;
