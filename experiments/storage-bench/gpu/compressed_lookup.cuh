// O1/O2/O5 share the exact fenced lookup; output positions are oracle-checked.
__global__ void lookup_kernel(RunView run,const Code*probes,size_t n,Code*out){for(size_t i=blockIdx.x*blockDim.x+threadIdx.x;i<n;i+=gridDim.x*blockDim.x)out[i]=Code(locate(run,probes[i]));}
__global__ void compare_codes(const Code*out,const Code*ref,size_t n,unsigned*err){for(size_t i=blockIdx.x*blockDim.x+threadIdx.x;i<n;i+=gridDim.x*blockDim.x)if(out[i]!=ref[i])atomicExch(err,1u);}
bool check_codes(Buf&out,Buf&ref,size_t n,Buf&err){CK(cudaMemset(err.p,0,4));compare_codes<<<1024,256>>>(out.as<Code>(),ref.as<Code>(),n,err.as<unsigned>());unsigned e;CK(cudaMemcpy(&e,err.p,4,cudaMemcpyDeviceToHost));return !e;}
std::vector<Code> dataset(const std::string& name){
 std::vector<Code>v;constexpr size_t n=16000000;U state=9127;
 if(name=="orders")v=load_codes("orders.o_orderkey");else if(name=="parts")v=load_codes("lineitem.l_partkey");else if(name=="customers")v=load_codes("orders.o_custkey");
 else {v.resize(n);for(size_t i=0;i<n;i++){if(name=="dense")v[i]=i*11/10;else if(name=="clustered")v[i]=(i/1024)*65536+i%1024;else if(name=="sparse")v[i]=rng(state)&0x7fffffff;else exit(1);}}
 std::sort(v.begin(),v.end());v.erase(std::unique(v.begin(),v.end()),v.end());
 if(name=="sparse")while(v.size()<n){size_t missing=n-v.size();for(size_t j=0;j<missing;j++)v.push_back(rng(state)&0x7fffffff);std::sort(v.begin(),v.end());v.erase(std::unique(v.begin(),v.end()),v.end());}
 return v;
}
std::vector<Code> probes_for(const std::vector<Code>&keys,size_t n,int pct,bool ordered){
 std::vector<Code>p(n);U state=81723;size_t hits=n*pct/100;bool holes=U(keys.back())-keys.front()+1>keys.size()*1.01;
 for(size_t i=0;i<n;i++){if(i<hits)p[i]=keys[rng(state)%keys.size()];else{Code x;if(holes){do{x=keys.front()+rng(state)%(U(keys.back())-keys.front()+1);}while(std::binary_search(keys.begin(),keys.end(),x));}else x=keys.back()+1+Code(rng(state)%1000000);p[i]=x;}}
 if(ordered)std::sort(p.begin(),p.end());else{std::mt19937 g(811);std::shuffle(p.begin(),p.end(),g);}return p;
}
void loading_record(const std::string&caseid,std::chrono::steady_clock::time_point start){std::cout<<"{\"type\":\"load_encode\",\"case\":"<<esc(caseid)<<",\"wall_ms\":"<<std::chrono::duration<double,std::milli>(std::chrono::steady_clock::now()-start).count()<<"}\n";}
void do_lookup(const std::string&name,int pct,bool ordered){
 auto start=std::chrono::steady_clock::now();auto keys=dataset(name);auto probes=probes_for(keys,4000000,pct,ordered);std::vector<Code>expected(probes.size());for(size_t i=0;i<probes.size();i++){auto it=std::lower_bound(keys.begin(),keys.end(),probes[i]);expected[i]=it!=keys.end()&&*it==probes[i]?it-keys.begin():keys.size();}
 std::string id=name+"_h"+std::to_string(pct)+(ordered?"_sorted":"_random");std::vector<Run>runs;for(int k=0;k<5;k++)runs.push_back(make_run(keys,k));bool bitmap=U(keys.back())-keys.front()+1<=32*keys.size();if(bitmap)runs.push_back(make_run(keys,5));else std::cout<<"{\"type\":\"not_applicable\",\"experiment\":\"O1\",\"case\":"<<esc(id)<<",\"variant\":\"V5_bitmap_rank\",\"reason\":\"span exceeds 32*n\"}\n";
 auto dp=upload(probes),ref=upload(expected);Buf out(probes.size()*4),err(4);loading_record(id,start);std::vector<Variant>vs;
 for(size_t k=0;k<runs.size();k++){auto*r=&runs[k];metadata("O1",id,run_names[k],k==0?"L2":"L1",r->bytes(),keys.size());vs.push_back({run_names[k],r->bytes(),0,[&,r]{lookup_kernel<<<4096,256>>>(r->view(),dp.as<Code>(),probes.size(),out.as<Code>());},[&]{return check_codes(out,ref,probes.size(),err);}});}
 vs.push_back(vs[1]);vs.back().name="V1_noise";metadata("O1",id,"V1_noise","L1",runs[1].bytes(),keys.size());
 bench("O1",id,keys.size(),vs,",\"probe_count\":"+std::to_string(probes.size())+",\"hit_percent\":"+std::to_string(pct)+",\"probe_order\":"+esc(ordered?"sorted":"random")+",\"span\":"+std::to_string(U(keys.back())-keys.front()+1)+",\"miss_distribution\":"+esc(U(keys.back())-keys.front()+1>keys.size()*1.01?"uniform holes within span":"outside span: no substantial holes"));
}
__global__ void expand_kernel(RunView keys,RunView offsets,RunView payload,const Code*probes,size_t n,U*out){for(size_t i=blockIdx.x*blockDim.x+threadIdx.x;i<n;i+=gridDim.x*blockDim.x){size_t p=locate(keys,probes[i]);U sum=0;if(p<keys.n){Code end=code_at(offsets,p+1);for(Code j=code_at(offsets,p);j<end;j++)sum+=code_at(payload,j);}out[i]=sum;}}
__global__ void compare_wide(const U*out,const U*ref,size_t n,unsigned*err){for(size_t i=blockIdx.x*blockDim.x+threadIdx.x;i<n;i+=gridDim.x*blockDim.x)if(out[i]!=ref[i])atomicExch(err,1u);}
bool check_wide(Buf&out,Buf&ref,size_t n,Buf&err){CK(cudaMemset(err.p,0,4));compare_wide<<<1024,256>>>(out.as<U>(),ref.as<U>(),n,err.as<unsigned>());unsigned e;CK(cudaMemcpy(&e,err.p,4,cudaMemcpyDeviceToHost));return !e;}
void do_expand(){
 auto start=std::chrono::steady_clock::now();auto order=load_codes("lineitem.l_orderkey"),payload=load_codes("lineitem.l_partkey");if(order.size()!=payload.size()||!std::is_sorted(order.begin(),order.end())){std::cerr<<"lineitem keys must be grouped in source\n";exit(5);}
 std::vector<Code>keys,offsets;std::vector<U>sums;for(size_t i=0;i<order.size();i++){if(!i||order[i]!=order[i-1]){keys.push_back(order[i]);offsets.push_back(i);sums.push_back(0);}sums.back()+=payload[i];}offsets.push_back(order.size());order.clear();order.shrink_to_fit();auto probes=probes_for(keys,4000000,50,false);std::vector<U>expected(probes.size());for(size_t i=0;i<probes.size();i++){auto it=std::lower_bound(keys.begin(),keys.end(),probes[i]);if(it!=keys.end()&&*it==probes[i])expected[i]=sums[it-keys.begin()];}
 std::vector<Run>kr,orr,pr;for(int k=0;k<6;k++)kr.push_back(make_run(keys,k));for(int k:{1,3,4})orr.push_back(make_run(offsets,k));for(int k:{1,3})pr.push_back(make_run(payload,k));auto dp=upload(probes),ref=upload(expected);Buf out(probes.size()*8),err(4);loading_record("sf10_order_part",start);std::vector<Variant>vs;
 auto add=[&](int k,int o,int p,std::string name){auto*a=&kr[k];auto*b=&orr[o];auto*c=&pr[p];size_t bytes=a->bytes()+b->bytes()+c->bytes();metadata("O2","sf10_order_part",name,k==0?"L2":"L1",bytes,keys.size());vs.push_back({name,bytes,0,[&,a,b,c]{expand_kernel<<<4096,256>>>(a->view(),b->view(),c->view(),dp.as<Code>(),probes.size(),out.as<U>());},[&]{return check_wide(out,ref,probes.size(),err);}});};
 for(int k=0;k<6;k++)add(k,0,0,run_names[k]);for(int o=0;o<3;o++)for(int p=0;p<2;p++)if(o||p)add(1,o,p,std::string("V1_offsets_")+(o==0?"u32":o==1?"for":"delta")+"_payload_"+(p?"for":"u32"));add(3,1,1,"V3_offsets_for_payload_for");add(3,2,1,"V3_offsets_delta_payload_for");add(1,0,0,"V1_noise");
 bench("O2","sf10_order_part",keys.size(),vs,",\"probe_count\":4000000,\"hit_percent\":50,\"payload_rows\":"+std::to_string(payload.size()));
}
__global__ void admission_kernel(RunView run,const Code*batch,size_t n,U*out){U fresh=0;for(size_t i=blockIdx.x*blockDim.x+threadIdx.x;i<n;i+=gridDim.x*blockDim.x)fresh+=locate(run,batch[i])==run.n;blocksum(fresh,out);}
void do_admission(size_t n){
 auto start=std::chrono::steady_clock::now();std::vector<Code>keys(16000000),batch(n);for(size_t i=0;i<keys.size();i++)keys[i]=2*i;for(size_t i=0;i<n/2;i++){batch[2*i]=2*i;batch[2*i+1]=2*i+1;}U oracle=0;for(Code x:batch)oracle+=!std::binary_search(keys.begin(),keys.end(),x);
 std::string id="run16M_batch"+std::to_string(n);std::vector<Run>runs;for(int k=0;k<6;k++)runs.push_back(make_run(keys,k));auto db=upload(batch);Buf out(8);loading_record(id,start);std::vector<Variant>vs;
 for(int k=0;k<6;k++){auto*r=&runs[k];metadata("O5",id,run_names[k],k==0?"L2":"L1",r->bytes(),keys.size());vs.push_back({run_names[k],r->bytes(),0,[&,r]{CK(cudaMemsetAsync(out.p,0,8));admission_kernel<<<4096,256>>>(r->view(),db.as<Code>(),batch.size(),out.as<U>());},[&]{return scalarcheck(out,oracle);}});}vs.push_back(vs[1]);vs.back().name="V1_noise";metadata("O5",id,"V1_noise","L1",runs[1].bytes(),keys.size());bench("O5",id,keys.size(),vs,",\"batch_count\":"+std::to_string(n)+",\"fresh_count\":"+std::to_string(oracle));
}
