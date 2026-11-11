#include "compressed_common.cuh"
#include "compressed_lookup.cuh"
#include "compressed_merge_sort.cuh"
#include "compressed_semijoin.cuh"
int main(int argc,char**argv){
 if(argc<2)return 1;CK(cudaSetDevice(0));
 std::cout<<"{\"type\":\"environment\",\"pid\":"<<getpid()<<",\"compiler\":\"nvcc -O3 -std=c++17 -arch=sm_89\",\"gpu\":"<<esc(cmd("nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv"))<<"}\n"<<std::flush;
 std::string mode=argv[1];
 if(mode=="lookup"&&argc==5)do_lookup(argv[2],atoi(argv[3]),std::string(argv[4])=="sorted");
 else if(mode=="expand")do_expand();
 else if(mode=="admission"&&argc==3)do_admission(std::stoull(argv[2]));
 else if(mode=="merge"&&argc==3)do_merge(argv[2]);
 else if(mode=="sort")do_sort();
 else if(mode=="semijoin"&&argc==3)do_semijoin(atoi(argv[2]));
 else if(mode=="setops"&&argc==3)do_setops(atoi(argv[2]));else return 1;
 CK(cudaDeviceSynchronize());return 0;
}
