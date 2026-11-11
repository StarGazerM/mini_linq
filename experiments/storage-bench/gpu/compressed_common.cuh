#pragma once
#define STORAGE_BENCH_LIBRARY
#include "bench.cu"
#undef STORAGE_BENCH_LIBRARY
#include <cub/device/device_radix_sort.cuh>
#include <cub/device/device_select.cuh>
#include <limits>
using Code=uint32_t;
// Offsets are word offsets. FOR descriptors cover 1024 codes; delta descriptors
// cover 64 codes. Fences always cover 1024 sorted keys.
struct Frame {Code base,maximum,offset,width;};
struct RunView {const Code* words;const U* wide;const Frame* frames;const Code* fences;const Code* rank;size_t n;Code minimum,maximum;int kind;};
struct Run {
 int kind;size_t n;Code minimum,maximum;std::vector<Code> words,fences,prefix;std::vector<U>wide;std::vector<Frame>frames;
 Buf dw,du,df,dfe,dr;
 RunView view()const{return {dw.as<Code>(),du.as<U>(),df.as<Frame>(),dfe.as<Code>(),dr.as<Code>(),n,minimum,maximum,kind};}
 size_t bytes()const{return dw.n+du.n+df.n+dfe.n+dr.n;}
};
std::vector<Code> load_codes(const std::string& column,int native=8){
 std::string path="/data/tpch-gpu/sf10/"+column+".bin";
 std::ifstream f(path,std::ios::binary|std::ios::ate);if(!f){std::cerr<<path<<" unavailable\n";exit(5);}size_t bytes=f.tellg();if(bytes%native)exit(5);f.seekg(0);size_t n=bytes/native;std::vector<Code>v(n);
 if(native==4)f.read((char*)v.data(),bytes);else{std::vector<U>chunk(1<<20);for(size_t st=0;st<n;st+=chunk.size()){size_t take=std::min(chunk.size(),n-st);f.read((char*)chunk.data(),take*8);for(size_t j=0;j<take;j++){if(chunk[j]>UINT32_MAX)exit(5);v[st+j]=chunk[j];}}}if(!f)exit(5);return v;
}
Run make_run(const std::vector<Code>&v,int kind){
 Run c;c.kind=kind;c.n=v.size();auto mm=std::minmax_element(v.begin(),v.end());c.minimum=*mm.first;c.maximum=*mm.second;
 if(kind==0)c.wide.assign(v.begin(),v.end());else if(kind==1)c.words=v;else if(kind==5){
  size_t span=U(c.maximum)-c.minimum+1;c.words.resize((span+31)/32);for(Code x:v)c.words[(x-c.minimum)/32]|=1u<<((x-c.minimum)%32);
  c.prefix.resize(c.words.size());Code s=0;for(size_t i=0;i<c.words.size();i++){c.prefix[i]=s;s+=__builtin_popcount(c.words[i]);}
 }else{
  size_t block=kind==4?64:1024;
  for(size_t st=0;st<v.size();st+=1024)c.fences.push_back(v[st]);
  for(size_t st=0;st<v.size();st+=block){size_t en=std::min(st+block,v.size());auto m=std::minmax_element(v.begin()+st,v.begin()+en);Code base=kind==4?v[st]:*m.first,range=*m.second-base;
   if(kind==4){range=0;for(size_t j=st+1;j<en;j++)range=std::max(range,v[j]-v[j-1]);}
   unsigned w=bits(range);if(kind==2)w=w<=8?8:w<=16?16:32;
   size_t off=c.words.size();c.frames.push_back({base,*m.second,Code(off),w});c.words.resize(off+(block*w+31)/32,0);
   for(size_t j=st;j<en&&w;j++){Code x=kind==4?(j==st?0:v[j]-v[j-1]):v[j]-base;size_t bit=(j-st)*w,word=off+bit/32;unsigned shift=bit%32;c.words[word]|=x<<shift;if(shift+w>32)c.words[word+1]|=x>>(32-shift);}
  }
 }
 c.dw=upload(c.words);c.du=upload(c.wide);c.df=upload(c.frames);c.dfe=upload(c.fences);c.dr=upload(c.prefix);return c;
}
__device__ __forceinline__ Code residual(RunView v,Frame f,size_t j){unsigned w=f.width;if(!w)return 0;if(v.kind==2){const Code*p=v.words+f.offset;return w==8?reinterpret_cast<const uint8_t*>(p)[j]:w==16?reinterpret_cast<const uint16_t*>(p)[j]:p[j];}size_t bit=j*w,word=f.offset+bit/32;unsigned shift=bit%32;Code x=v.words[word]>>shift;if(shift+w>32)x|=v.words[word+1]<<(32-shift);return w==32?x:x&((1u<<w)-1);}
__device__ __forceinline__ Code code_at(RunView v,size_t i){if(v.kind==0)return v.wide[i];if(v.kind==1)return v.words[i];if(v.kind==4){Frame f=v.frames[i/64];Code x=f.base;for(size_t j=1;j<=i%64;j++)x+=residual(v,f,j);return x;}Frame f=v.frames[i/1024];return f.base+residual(v,f,i%1024);}
__device__ __forceinline__ size_t locate(RunView v,Code key){
 if(key<v.minimum||key>v.maximum)return v.n;
 if(v.kind==5){Code k=key-v.minimum,w=v.words[k/32],mask=1u<<(k%32);return (w&mask)?v.rank[k/32]+__popc(w&(mask-1)):v.n;}
 size_t lo=0,hi=v.n;
 if(v.kind>=2){size_t l=0,h=(v.n+1023)/1024;while(l<h){size_t m=(l+h)/2;if(v.fences[m]<=key)l=m+1;else h=m;}size_t block=l-1;lo=block*1024;hi=min(lo+1024,v.n);
  if(v.kind==4){size_t l=lo/64,h=(hi+63)/64;while(l<h){size_t m=(l+h)/2;if(v.frames[m].base<=key)l=m+1;else h=m;}lo=(l-1)*64;hi=min(lo+64,v.n);Frame f=v.frames[lo/64];Code x=f.base;for(size_t i=lo;i<hi;i++){if(i>lo)x+=residual(v,f,i-lo);if(x==key)return i;if(x>key)break;}return v.n;}
  Frame f=v.frames[block];Code r=key-f.base;if(key<f.base||(f.width<32&&U(r)>=(1ULL<<f.width)))return v.n;size_t end=hi;while(lo<hi){size_t m=(lo+hi)/2;if(residual(v,f,m%1024)<r)lo=m+1;else hi=m;}return lo<end&&residual(v,f,lo%1024)==r?lo:v.n;
 }
 while(lo<hi){size_t m=(lo+hi)/2;if(code_at(v,m)<key)lo=m+1;else hi=m;}return lo<v.n&&code_at(v,lo)==key?lo:v.n;
}
__global__ void check_run_kernel(RunView v,const Code*ref,size_t n,unsigned*err){for(size_t i=blockIdx.x*blockDim.x+threadIdx.x;i<n;i+=gridDim.x*blockDim.x)if(code_at(v,i)!=ref[i])atomicExch(err,1u);}
bool check_run(RunView v,Buf&ref,size_t n,Buf&err){CK(cudaMemset(err.p,0,4));check_run_kernel<<<1024,256>>>(v,ref.as<Code>(),n,err.as<unsigned>());unsigned e;CK(cudaMemcpy(&e,err.p,4,cudaMemcpyDeviceToHost));return e==0;}
void metadata(const std::string&op,const std::string&caseid,const std::string&variant,const std::string&level,size_t bytes,size_t n){std::cout<<"{\"type\":\"layout\",\"experiment\":"<<esc(op)<<",\"case\":"<<esc(caseid)<<",\"variant\":"<<esc(variant)<<",\"level\":"<<esc(level)<<",\"stored_bytes\":"<<bytes<<",\"bytes_per_key\":"<<double(bytes)/n<<"}\n";}
const char*run_names[]={"V0_u64","V1_u32","V2_bytes","V3_for","V4_delta","V5_bitmap_rank"};
void do_merge(const std::string& regime);
void do_sort();
void do_semijoin(int pct);
void do_setops(int pct);
