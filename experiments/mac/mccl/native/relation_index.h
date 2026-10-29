#ifndef MCCL_NATIVE_RELATION_INDEX_H
#define MCCL_NATIVE_RELATION_INDEX_H

#include "mccl.h"
#include "mlx/array.h"
#include "mlx/stream.h"
#include <vector>

namespace mccl::native {

// Internal diagnostic interface, not a C/Rust public storage API. Outputs are
// {capacity_rows, directory, metadata}; capacity_rows may have an uninitialized
// tail and MUST NOT be published or read directly. All outputs share one MLX
// multi-output primitive. An observer must preserve that sibling relationship
// with array::make_arrays, not wrap each output as an independent primitive.
MCCL_API std::vector<mlx::core::array> make_relation_index_graph(
    const mlx::core::array& relation, int key_column, int payload_column,
    const mlx::core::array* selection, mlx::core::Stream stream,
    bool build_directory = true);

struct RelationIndexResult {
  mlx::core::array rows;
  mlx::core::array directory;
  size_t count;
};

// Evaluate the complete dependency graph, inspect only GPU scalar metadata,
// reject invalid selected i64 values, and share ownership of the initialized
// row prefix. The returned arrays are ready and safe for CPU reads. Throws on
// errors; no partial result is returned. Does not scan/copy/populate row bytes.
MCCL_API RelationIndexResult finish_relation_index_graph(
    std::vector<mlx::core::array>& graph);

} // namespace mccl::native
#endif
