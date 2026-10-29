#include <cub/device/device_scan.cuh>
#include <cuda_profiler_api.h>
#include <cuda_runtime.h>
#include <algorithm>
#include <array>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <functional>
#include <iostream>
#include <numeric>
#include <random>
#include <sstream>
#include <string>
#include <thread>
#include <vector>
#include <unistd.h>
using U=unsigned long long;
#define CK(x) do {auto cuda_status_=(x);if(cuda_status_!=cudaSuccess){fprintf(stderr,"%s:%d %s\n",__FILE__,__LINE__,cudaGetErrorString(cuda_status_));exit(2);}}while(0)
std::string cmd(const char* c){std::string s;char b[4096];FILE*f=popen(c,"r");if(!f)exit(3);while(fgets(b,sizeof b,f))s+=b;int r=pclose(f);if(r)exit(3);return s;}
std::string esc(const std::string&s){std::string r="\"";for(char c:s){if(c=='\n')r+="\\n";else if(c=='\"'||c=='\\'){r+='\\';r+=c;}else if(c=='\r')r+="\\r";else r+=c;}return r+'"';}
std::string tenants(){return cmd("nvidia-smi --query-compute-apps=pid,used_memory --format=csv,noheader");}
bool foreign(const std::string&s){std::istringstream in(s);std::string l;while(std::getline(in,l))if(!l.empty()&&std::strtol(l.c_str(),nullptr,10)!=getpid())return true;return false;}
struct Buf {void*p=nullptr;size_t n=0;Buf()=default;explicit Buf(size_t z):n(z){if(z)CK(cudaMalloc(&p,z));}~Buf(){if(p)cudaFree(p);}Buf(const Buf&)=delete;Buf&operator=(const Buf&)=delete;Buf(Buf&&b):p(b.p),n(b.n){b.p=nullptr;}Buf&operator=(Buf&&b){if(p)cudaFree(p);p=b.p;n=b.n;b.p=nullptr;return *this;}template<class T>T*as()const{return (T*)p;}};
template<class T>Buf upload(const std::vector<T>&v){Buf b(v.size()*sizeof(T));if(b.n)CK(cudaMemcpy(b.p,v.data(),b.n,cudaMemcpyHostToDevice));return b;}
struct View {const void*data;const U*base;const U*offset;const unsigned char*width;const U*dict;int kind,native;};
struct Col {Buf data,base,offset,width,dict;std::vector<unsigned char> host;std::vector<U> hb,ho,hd;std::vector<unsigned char>hw;int kind,native;size_t n;View view()const{return {data.p,base.as<U>(),offset.as<U>(),width.as<unsigned char>(),dict.as<U>(),kind,native};}size_t bytes()const{return data.n+base.n+offset.n+width.n+dict.n;}};
int bits(U x){return x?64-__builtin_clzll(x):0;}
Col encode(const std::vector<U>&v,int kind,int native=8){Col c;c.kind=kind;c.native=native;c.n=v.size();if(kind==0){c.host.resize(v.size()*native);for(size_t i=0;i<v.size();i++)memcpy(c.host.data()+i*native,&v[i],native);}else{std::vector<U>codes;const std::vector<U>*src=&v;int dw=0;if(kind>=3){std::vector<std::pair<U,uint32_t>> order(v.size());for(size_t i=0;i<v.size();i++)order[i]={v[i],uint32_t(i)};std::sort(order.begin(),order.end());codes.resize(v.size());for(auto p:order){if(c.hd.empty()||c.hd.back()!=p.first)c.hd.push_back(p.first);codes[p.second]=c.hd.size()-1;}dw=bits(c.hd.size()-1);src=&codes;}
for(size_t st=0;st<v.size();st+=1024){size_t en=std::min(st+1024,v.size());auto mm=std::minmax_element(src->begin()+st,src->begin()+en);U base=kind>=3?0:*mm.first;int w=kind>=3?dw:bits(*mm.second-base);if(kind==1||kind==4)w=w<=8?8:w<=16?16:32;if(w>32){fprintf(stderr,"unsupported width %d\n",w);exit(4);}c.hb.push_back(base);c.hw.push_back(w);c.ho.push_back(c.host.size());size_t bytes=(kind==1||kind==4)?1024*w/8:32*((32*w+31)/32)*4;size_t off=c.host.size();c.host.resize(off+bytes,0);for(size_t i=st;i<en;i++){U x=(*src)[i]-base;size_t j=i-st;if(kind==1||kind==4)memcpy(c.host.data()+off+j*(w/8),&x,w/8);else if(w){size_t bit=(j/32)*w,word=(bit/32)*32+j%32;uint32_t low=uint32_t(x<<(bit%32));uint32_t a;memcpy(&a,c.host.data()+off+word*4,4);a|=low;memcpy(c.host.data()+off+word*4,&a,4);if(bit%32+w>32){uint32_t hi=uint32_t(x>>(32-bit%32));memcpy(&a,c.host.data()+off+(word+32)*4,4);a|=hi;memcpy(c.host.data()+off+(word+32)*4,&a,4);}}}}
c.base=upload(c.hb);c.offset=upload(c.ho);c.width=upload(c.hw);c.dict=upload(c.hd);}c.data=upload(c.host);return c;}
__device__ __forceinline__ U get(View c,size_t i){if(c.kind==0)return c.native==4?((const uint32_t*)c.data)[i]:((const U*)c.data)[i];size_t b=i/1024,j=i%1024;int w=c.width[b];const unsigned char*p=(const unsigned char*)c.data+c.offset[b];U x=0;if(c.kind==1||c.kind==4)x=w==8?p[j]:w==16?((const uint16_t*)p)[j]:((const uint32_t*)p)[j];else if(w){size_t bit=(j/32)*w,word=(bit/32)*32+j%32;U a=((const uint32_t*)p)[word];if(bit%32+w>32)a|=U(((const uint32_t*)p)[word+32])<<32;x=(a>>(bit%32))&((1ULL<<w)-1);}return c.kind>=3?c.dict[x]:x+c.base[b];}
__device__ void blocksum(U x,U*out){__shared__ U s[256];s[threadIdx.x]=x;__syncthreads();for(int d=128;d;d/=2){if(threadIdx.x<d)s[threadIdx.x]+=s[threadIdx.x+d];__syncthreads();}if(!threadIdx.x)atomicAdd(out,s[0]);}
__global__ void countk(View c,size_t n,U threshold,U*out){U sum=0;for(size_t i=blockIdx.x*blockDim.x+threadIdx.x;i<n;i+=gridDim.x*blockDim.x)sum+=get(c,i)<threshold;blocksum(sum,out);}
__global__ void q6k(View date,View disc,View qty,View price,size_t n,U*out){U sum=0;for(size_t i=blockIdx.x*blockDim.x+threadIdx.x;i<n;i+=gridDim.x*blockDim.x){U d=get(date,i);if(d>=8766&&d<9131){U x=get(disc,i);if(x>=5&&x<=7&&get(qty,i)<2400)sum+=get(price,i)*x;}}blocksum(sum,out);}
__global__ void decodek(View c,size_t n,U*out){for(size_t i=blockIdx.x*blockDim.x+threadIdx.x;i<n;i+=gridDim.x*blockDim.x)out[i]=get(c,i);}
__global__ void checkk(View c,const U*ref,size_t n,unsigned int*err){for(size_t i=blockIdx.x*blockDim.x+threadIdx.x;i<n;i+=gridDim.x*blockDim.x)if(get(c,i)!=ref[i])atomicExch(err,1);}
template<class O>__global__ void probek(const U*keys,size_t nk,const O*offset,View payload,const U*probes,U*out){size_t i=blockIdx.x*blockDim.x+threadIdx.x;if(i>=65536)return;U key=probes[i];size_t l=0,h=nk;while(l<h){size_t m=(l+h)/2;if(keys[m]<key)l=m+1;else h=m;}U sum=0;if(l<nk&&keys[l]==key)for(size_t j=offset[l];j<offset[l+1];j++)sum+=get(payload,j);out[i]=sum;}
// Merge-path: each thread co-ranks the start of its eight-output stripe.
__device__ size_t corank(size_t k,const U*a,size_t na,const U*b,size_t nb){size_t lo=k>nb?k-nb:0,hi=k<na?k:na;while(lo<hi){size_t i=(lo+hi)/2,j=k-i;if(j>0&&i<na&&b[j-1]>=a[i])lo=i+1;else hi=i;}return lo;}
__global__ void mergek(const U*a,size_t na,const U*b,size_t nb,U*out){size_t k=(size_t(blockIdx.x)*blockDim.x+threadIdx.x)*8;if(k>=na+nb)return;size_t i=corank(k,a,na,b,nb),j=k-i;for(int t=0;t<8&&k<na+nb;t++,k++){if(i<na&&(j==nb||a[i]<=b[j]))out[k]=a[i++];else out[k]=b[j++];}}
// Compact re-encoding: metadata, exclusive scan of block byte sizes, then packing.
__global__ void encmeta(const U*in,size_t n,U*base,U*sizes,unsigned char*width){size_t b=blockIdx.x*blockDim.x+threadIdx.x,st=b*1024;if(st>=n)return;U lo=in[st],hi=in[min(st+1023,n-1)];int w=hi==lo?0:64-__clzll(hi-lo);base[b]=lo;width[b]=w;sizes[b]=128*w;}
__global__ void encodek(const U*in,size_t n,const U*base,const U*offset,const unsigned char*width,uint32_t*out){size_t b=blockIdx.x,st=b*1024;U lo=base[b];int w=width[b],lane=threadIdx.x;for(int word=0;word<w;word++){U z=0;int first=word*32/w,last=min(31,((word+1)*32-1)/w);for(int j=first;j<=last;j++){size_t ix=st+j*32+lane;U x=ix<n?in[ix]-lo:0;int shift=j*w-word*32;if(shift>=0)z|=x<<shift;else z|=x>>(-shift);}out[offset[b]/4+word*32+lane]=uint32_t(z);}}
std::vector<U> load(const std::string&name,int native=8){std::ifstream f("/data/mini_linq/target/storage-bench/tpch-sf1/"+name+".bin",std::ios::binary|std::ios::ate);if(!f)exit(5);size_t n=f.tellg()/native;f.seekg(0);std::vector<U>v(n);if(native==8)f.read((char*)v.data(),n*8);else{std::vector<uint32_t>x(n);f.read((char*)x.data(),n*4);std::copy(x.begin(),x.end(),v.begin());}return v;}
U rng(U&i){U z=(i+=0x9e3779b97f4a7c15ULL);z=(z^(z>>30))*0xbf58476d1ce4e5b9ULL;z=(z^(z>>27))*0x94d049bb133111ebULL;return z^(z>>31);}
struct Variant{std::string name;size_t stored;double readbytes;std::function<void()>run;std::function<bool()>check;};
const char*names[]={"plain","bytes","for","dict","dict_bytes"};
void bench(const std::string&experiment,const std::string&caseid,size_t n,const std::vector<Variant>&vs,const std::string&extra=""){cudaEvent_t a,b;CK(cudaEventCreate(&a));CK(cudaEventCreate(&b));std::string clocks=cmd("nvidia-smi -q -d CLOCK");std::cout<<"{\"type\":\"group\",\"experiment\":"<<esc(experiment)<<",\"case\":"<<esc(caseid)<<",\"n\":"<<n<<",\"clocks\":"<<esc(clocks)<<",\"load\":"<<esc(cmd("cat /proc/loadavg"))<<extra<<"}\n"<<std::flush;
bool profiling=std::getenv("BENCH_PROFILE")!=nullptr;
for(int r=-5;r<(profiling?1:21);r++)for(size_t vi=0;vi<vs.size();vi++){auto&v=vs[(vi+(r+5)%vs.size())%vs.size()];int retries=0;for(;;){std::string before=tenants();if(foreign(before)){std::cout<<"{\"type\":\"tenant_wait\",\"case\":"<<esc(caseid)<<",\"record\":"<<esc(before)<<"}\n"<<std::flush;std::this_thread::sleep_for(std::chrono::seconds(5));if(++retries>120)exit(7);continue;}if(profiling&&r==0)CK(cudaProfilerStart());CK(cudaEventRecord(a));v.run();CK(cudaEventRecord(b));CK(cudaEventSynchronize(b));float ms;CK(cudaEventElapsedTime(&ms,a,b));if(profiling&&r==0)CK(cudaProfilerStop());std::string after=tenants();bool ok=v.check();std::cout<<"{\"type\":\"sample\",\"experiment\":"<<esc(experiment)<<",\"case\":"<<esc(caseid)<<",\"variant\":"<<esc(v.name)<<",\"n\":"<<n<<",\"round\":"<<r<<",\"ms\":"<<ms<<",\"stored_bytes\":"<<v.stored<<",\"logical_read_bytes\":"<<v.readbytes<<",\"exact\":"<<(ok?"true":"false")<<",\"valid\":"<<(!foreign(after)?"true":"false")<<",\"tenants_before\":"<<esc(before)<<",\"tenants_after\":"<<esc(after)<<"}\n"<<std::flush;if(!ok)exit(6);if(!foreign(after))break;if(++retries>120)exit(7);}}CK(cudaEventDestroy(a));CK(cudaEventDestroy(b));}
bool scalarcheck(Buf&out,U ref){U got;CK(cudaMemcpy(&got,out.p,8,cudaMemcpyDeviceToHost));return got==ref;}
bool arraycheck(View out,Buf&ref,size_t n,Buf&err){CK(cudaMemset(err.p,0,4));checkk<<<1024,256>>>(out,ref.as<U>(),n,err.as<unsigned int>());unsigned int e;CK(cudaMemcpy(&e,err.p,4,cudaMemcpyDeviceToHost));return !e;}
void sweep(size_t n,int w){std::vector<U>v(n);U state=1234,mask=(1ULL<<w)-1;for(auto&x:v)x=rng(state)&mask;std::vector<Col>cols;for(int k=0;k<5;k++)cols.push_back(encode(v,k));Buf out(8);for(int pct:{1,15,50}){U threshold=((1ULL<<w)*pct)/100,ref=0;for(auto x:v)ref+=x<threshold;std::vector<Variant>vs;for(int k=0;k<5;k++){auto*c=&cols[k];vs.push_back({names[k],c->bytes(),double(c->bytes()),[&,c,threshold]{CK(cudaMemsetAsync(out.p,0,8));countk<<<1024,256>>>(c->view(),n,threshold,out.as<U>());},[&,ref]{return scalarcheck(out,ref);}});}bench("M1_count","w"+std::to_string(w)+"_n"+std::to_string(n)+"_s"+std::to_string(pct),n,vs,",\"threshold\":"+std::to_string(threshold)+",\"selected\":"+std::to_string(ref)+",\"width\":"+std::to_string(w)+",\"requested_selectivity_pct\":"+std::to_string(pct));}}
void q6(int tile){std::array<std::vector<U>,4>v={load("l_shipdate",4),load("l_discount"),load("l_quantity"),load("l_extendedprice")};size_t orig=v[0].size(),n=orig*tile;if(tile>1)for(auto&x:v){x.resize(n);for(size_t i=orig;i<n;i++)x[i]=x[i%orig];}U ref=0;std::array<size_t,4>visits={n,0,0,0};for(size_t i=0;i<n;i++)if(v[0][i]>=8766&&v[0][i]<9131){visits[1]++;if(v[1][i]>=5&&v[1][i]<=7){visits[2]++;if(v[2][i]<2400){visits[3]++;ref+=v[3][i]*v[1][i];}}}std::vector<std::array<Col,4>>cols(5);for(int k=0;k<5;k++)for(int j=0;j<4;j++)cols[k][j]=encode(v[j],k,j==0?4:8);Buf out(8);std::vector<Variant>vs;for(int k=0;k<5;k++){auto*c=&cols[k];size_t stored=0;double read=0;for(int j=0;j<4;j++){stored+=(*c)[j].bytes();read+=double((*c)[j].bytes())*visits[j]/n;}vs.push_back({names[k],stored,read,[&,c]{CK(cudaMemsetAsync(out.p,0,8));q6k<<<1024,256>>>((*c)[0].view(),(*c)[1].view(),(*c)[2].view(),(*c)[3].view(),n,out.as<U>());},[&]{return scalarcheck(out,ref);}});}bench("M1_q6","tpch_tile"+std::to_string(tile),n,vs,",\"reference_sum\":"+std::to_string(ref));}
void grouped(bool part){auto keys=load(part?"l_partkey":"l_orderkey");auto payload=part?load("l_suppkey"):std::vector<U>(keys.size());if(!part)std::iota(payload.begin(),payload.end(),0ULL);else{std::vector<std::pair<U,U>>p(keys.size());for(size_t i=0;i<p.size();i++)p[i]={keys[i],payload[i]};std::stable_sort(p.begin(),p.end(),[](auto a,auto b){return a.first<b.first;});for(size_t i=0;i<p.size();i++){keys[i]=p[i].first;payload[i]=p[i].second;}}std::vector<U>distinct,off;for(size_t i=0;i<keys.size();i++)if(i==0||keys[i]!=keys[i-1]){distinct.push_back(keys[i]);off.push_back(i);}off.push_back(keys.size());std::vector<uint32_t>off32(off.begin(),off.end());std::vector<U>probes(65536),expected(65536);U state=412;for(size_t i=0;i<probes.size();i++){size_t k=rng(state)%distinct.size();probes[i]=(i%2)?distinct.back()+1+(rng(state)%distinct.size()):distinct[k];if(i%2==0)for(size_t j=off[k];j<off[k+1];j++)expected[i]+=payload[j];}auto dk=upload(distinct),d32=upload(off32),d64=upload(off),dp=upload(probes),ref=upload(expected);Buf out(65536*8),err(4);std::vector<Col>cols;for(int k=0;k<3;k++)cols.push_back(encode(payload,k));std::vector<Variant>vs;for(int ow:{32,64})for(int k=0;k<3;k++){auto*c=&cols[k];vs.push_back({std::string(names[k])+"_offset"+std::to_string(ow),c->bytes()+(ow==32?d32.n:d64.n)+dk.n,0,[&,c,ow]{if(ow==32)probek<<<256,256>>>(dk.as<U>(),distinct.size(),d32.as<uint32_t>(),c->view(),dp.as<U>(),out.as<U>());else probek<<<256,256>>>(dk.as<U>(),distinct.size(),d64.as<U>(),c->view(),dp.as<U>(),out.as<U>());},[&]{return arraycheck({out.p,nullptr,nullptr,nullptr,nullptr,0,8},ref,65536,err);}});}bench("M2",part?"partkey_suppkey":"orderkey_row",65536,vs,",\"groups\":"+std::to_string(distinct.size())+",\"payload_rows\":"+std::to_string(payload.size())+",\"hit_fraction\":0.5");}
void merge(size_t na){
    size_t nb=na/10,n=na+nb,blocks=(n+1023)/1024;
    std::vector<U>a(na),b(nb),expected(n);
    for(size_t i=0;i<na;i++)a[i]=i*3;
    for(size_t i=0;i<nb;i++)b[i]=i*30+1;
    std::merge(a.begin(),a.end(),b.begin(),b.end(),expected.begin());
    size_t output_bytes=17*blocks;
    for(size_t st=0;st<n;st+=1024)output_bytes+=128*bits(expected[std::min(st+1023,n-1)]-expected[st]);
    auto pa=encode(a,0),pb=encode(b,0),ca=encode(a,2),cb=encode(b,2);
    auto ref=upload(expected);
    Buf da(na*8),db(nb*8),out(n*8),packed(blocks*4096),base(blocks*8),offset(blocks*8),sizes(blocks*8),width(blocks),err(4);
    size_t temp_bytes=0;
    CK(cub::DeviceScan::ExclusiveSum(nullptr,temp_bytes,sizes.as<U>(),offset.as<U>(),blocks));
    Buf temp(temp_bytes);
    View pv{out.p,nullptr,nullptr,nullptr,nullptr,0,8},cv{packed.p,base.as<U>(),offset.as<U>(),width.as<unsigned char>(),nullptr,2,8};
    std::vector<Variant>vs;
    vs.push_back({"plain",pa.bytes()+pb.bytes(),double(n*16),[&]{
        mergek<<<(n+2047)/2048,256>>>(pa.data.as<U>(),na,pb.data.as<U>(),nb,out.as<U>());
    },[&]{return arraycheck(pv,ref,n,err);}});
    vs.push_back({"for",ca.bytes()+cb.bytes(),double(ca.bytes()+cb.bytes()+n*32+output_bytes),[&]{
        decodek<<<1024,256>>>(ca.view(),na,da.as<U>());
        decodek<<<1024,256>>>(cb.view(),nb,db.as<U>());
        mergek<<<(n+2047)/2048,256>>>(da.as<U>(),na,db.as<U>(),nb,out.as<U>());
        encmeta<<<(blocks+255)/256,256>>>(out.as<U>(),n,base.as<U>(),sizes.as<U>(),width.as<unsigned char>());
        CK(cub::DeviceScan::ExclusiveSum(temp.p,temp_bytes,sizes.as<U>(),offset.as<U>(),blocks));
        encodek<<<blocks,32>>>(out.as<U>(),n,base.as<U>(),offset.as<U>(),width.as<unsigned char>(),packed.as<uint32_t>());
    },[&]{return arraycheck(cv,ref,n,err);}});
    bench("M3","merge_"+std::to_string(na)+"_"+std::to_string(nb),n,vs,",\"output_for_bytes\":"+std::to_string(output_bytes));
}
void transfer(size_t n,int w,bool real=false){std::vector<U>v;if(real){v=load("l_extendedprice");size_t orig=v.size();v.resize(n);for(size_t i=orig;i<n;i++)v[i]=v[i%orig];}else{v.resize(n);U state=1234;for(auto&x:v)x=rng(state)&((1ULL<<w)-1);}auto plain=encode(v,0),bytes=encode(v,1);auto ref=upload(v);Buf out(n*8),err(4);void*pp,*cp,*bp,*op,*wp;CK(cudaMallocHost(&pp,plain.host.size()));CK(cudaMallocHost(&cp,bytes.host.size()));CK(cudaMallocHost(&bp,bytes.hb.size()*8));CK(cudaMallocHost(&op,bytes.ho.size()*8));CK(cudaMallocHost(&wp,bytes.hw.size()));memcpy(pp,plain.host.data(),plain.host.size());memcpy(cp,bytes.host.data(),bytes.host.size());memcpy(bp,bytes.hb.data(),bytes.hb.size()*8);memcpy(op,bytes.ho.data(),bytes.ho.size()*8);memcpy(wp,bytes.hw.data(),bytes.hw.size());View ov{out.p,nullptr,nullptr,nullptr,nullptr,0,8};std::vector<Variant>vs={{"plain",plain.bytes(),double(plain.bytes()),[&]{CK(cudaMemcpyAsync(out.p,pp,n*8,cudaMemcpyHostToDevice));},[&]{return arraycheck(ov,ref,n,err);}},{"bytes",bytes.bytes(),double(bytes.bytes()),[&]{CK(cudaMemcpyAsync(bytes.data.p,cp,bytes.data.n,cudaMemcpyHostToDevice));CK(cudaMemcpyAsync(bytes.base.p,bp,bytes.base.n,cudaMemcpyHostToDevice));CK(cudaMemcpyAsync(bytes.offset.p,op,bytes.offset.n,cudaMemcpyHostToDevice));CK(cudaMemcpyAsync(bytes.width.p,wp,bytes.width.n,cudaMemcpyHostToDevice));decodek<<<1024,256>>>(bytes.view(),n,out.as<U>());},[&]{return arraycheck(ov,ref,n,err);}}};bench("M4",real?"price_n"+std::to_string(n):"w"+std::to_string(w)+"_n"+std::to_string(n),n,vs);CK(cudaFreeHost(pp));CK(cudaFreeHost(cp));CK(cudaFreeHost(bp));CK(cudaFreeHost(op));CK(cudaFreeHost(wp));}
int main(int argc,char**argv){if(argc<2)return 1;CK(cudaSetDevice(0));std::cout<<"{\"type\":\"environment\",\"pid\":"<<getpid()<<",\"compiler\":\"nvcc 13.0.48 -O3 -arch=sm_89 -std=c++17\",\"host_threads\":1,\"gpu\":"<<esc(cmd("nvidia-smi --query-gpu=name,driver_version,memory.total,pcie.link.gen.current,pcie.link.width.current --format=csv"))<<"}\n"<<std::flush;std::string mode=argv[1];if(mode=="sweep")sweep(std::stoull(argv[2]),atoi(argv[3]));else if(mode=="q6")q6(atoi(argv[2]));else if(mode=="group")grouped(atoi(argv[2]));else if(mode=="merge")merge(std::stoull(argv[2]));else if(mode=="transfer")transfer(std::stoull(argv[2]),atoi(argv[3]),argc>4);else return 1;CK(cudaDeviceSynchronize());
}
