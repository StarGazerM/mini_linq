#include <nvtx3/nvToolsExt.h>
#include <nvtx3/nvToolsExtCuda.h>

void cq_nvtx_push(const char *name) { nvtxRangePushA(name); }
void cq_nvtx_pop(void) { nvtxRangePop(); }
void cq_nvtx_stream(void *stream, const char *name) {
    nvtxNameCuStreamA((CUstream)stream, name);
}
