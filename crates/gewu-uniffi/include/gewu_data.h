// Read-only views of memory owned by UniFFI objects. See src/data.rs.
//
// Each function takes a handle obtained with the object's
// `uniffiCloneHandle()` and consumes it. The returned pointer stays valid,
// unchanged, while the caller keeps its own reference to that object.
#pragma once

#include <stddef.h>
#include <stdint.h>

/// The source row of each display row. When `rows` is NULL, display row `i`
/// is source row `i` for every `i < count`.
typedef struct {
    const uint32_t *rows;
    size_t count;
} GewuSourceRows;

typedef struct {
    const uint8_t *bytes;
    size_t count;
} GewuBytes;

/// Source rows of a `GewuOrderedView`, in display order.
GewuSourceRows gewu_ordered_view_source_rows(uint64_t view);

/// Contents of an `ExportedBytes`.
GewuBytes gewu_exported_bytes(uint64_t bytes);
