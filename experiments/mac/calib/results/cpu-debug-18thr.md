| op | proc | thr | footprint | GB/s | ns/item | median ms | gpu ms | notes |
|---|---|---:|---:|---:|---:|---:|---:|---|
| cpu_spawn | cpu | 12 | - | 0.0 | 15770.83 | 0.189 | - |  |
| cpu_spawn | cpu | 16 | - | 0.0 | 9541.62 | 0.153 | - |  |
| cpu_spawn | cpu | 17 | - | 0.0 | 10803.94 | 0.184 | - |  |
| cpu_spawn | cpu | 18 | - | 0.0 | 9486.11 | 0.171 | - |  |
| cpu_pool_dispatch | cpu | 12 | - | 0.0 | 1500.00 | 0.002 | - |  |
| cpu_pool_dispatch | cpu | 16 | - | 0.0 | 2075250.00 | 2.075 | - |  |
| cpu_pool_dispatch | cpu | 17 | - | 0.0 | 2078291.00 | 2.078 | - |  |
| cpu_pool_dispatch | cpu | 18 | - | 0.0 | 2080083.00 | 2.080 | - |  |
| seq_read | cpu | 12 | 4 MiB | 940.8 | 0.01 | 0.004 | - | elem_bytes=8 n_items=524288 unroll=8 |
| seq_read | cpu | 16 | 4 MiB | 2.0 | 3.97 | 2.082 | - | elem_bytes=8 n_items=524288 unroll=8 |
| seq_read | cpu | 17 | 4 MiB | 2.0 | 3.98 | 2.086 | - | elem_bytes=8 n_items=524288 unroll=8 |
| seq_read | cpu | 18 | 4 MiB | 2.0 | 3.98 | 2.086 | - | elem_bytes=8 n_items=524288 unroll=8 |
| seq_write | cpu | 12 | 4 MiB | 207.6 | 0.04 | 0.020 | - | elem_bytes=8 n_items=524288 |
| seq_write | cpu | 16 | 4 MiB | 2.0 | 3.97 | 2.083 | - | elem_bytes=8 n_items=524288 |
| seq_write | cpu | 17 | 4 MiB | 2.0 | 3.99 | 2.092 | - | elem_bytes=8 n_items=524288 |
| seq_write | cpu | 18 | 4 MiB | 2.0 | 3.99 | 2.093 | - | elem_bytes=8 n_items=524288 |
| seq_copy | cpu | 12 | 4 MiB | 397.9 | 0.04 | 0.021 | - | elem_bytes=8 n_items=524288 |
| seq_copy | cpu | 16 | 4 MiB | 4.0 | 4.01 | 2.101 | - | elem_bytes=8 n_items=524288 |
| seq_copy | cpu | 17 | 4 MiB | 4.0 | 4.00 | 2.096 | - | elem_bytes=8 n_items=524288 |
| seq_copy | cpu | 18 | 4 MiB | 4.0 | 4.00 | 2.096 | - | elem_bytes=8 n_items=524288 |
| gather_line | cpu | 12 | 4 MiB | 1623.8 | 0.08 | 0.003 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=32768 |
| gather_line | cpu | 16 | 4 MiB | 2.0 | 63.49 | 2.080 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=32768 |
| gather_line | cpu | 17 | 4 MiB | 2.0 | 63.48 | 2.080 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=32768 |
| gather_line | cpu | 18 | 4 MiB | 2.0 | 63.47 | 2.080 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=32768 |
| scatter_line | cpu | 12 | 4 MiB | 486.3 | 0.26 | 0.009 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=32768 |
| scatter_line | cpu | 16 | 4 MiB | 2.0 | 63.39 | 2.077 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=32768 |
| scatter_line | cpu | 17 | 4 MiB | 2.0 | 63.58 | 2.083 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=32768 |
| scatter_line | cpu | 18 | 4 MiB | 2.0 | 63.64 | 2.085 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=32768 |
| rmw_atomic | cpu | 12 | 4 MiB | 46.9 | 5.46 | 0.179 | - | bytes_per_item=256 elem_bytes=4 line_bytes=128 n_items=32768 |
| rmw_atomic | cpu | 16 | 4 MiB | 3.7 | 69.48 | 2.277 | - | bytes_per_item=256 elem_bytes=4 line_bytes=128 n_items=32768 |
| rmw_atomic | cpu | 17 | 4 MiB | 3.5 | 72.95 | 2.391 | - | bytes_per_item=256 elem_bytes=4 line_bytes=128 n_items=32768 |
| rmw_atomic | cpu | 18 | 4 MiB | 3.6 | 70.41 | 2.307 | - | bytes_per_item=256 elem_bytes=4 line_bytes=128 n_items=32768 |
| gather_dep | cpu | 12 | 4 MiB | 184.4 | 0.69 | 2.912 | - | line_bytes=128 n_items=4194304 steps_per_thread=349525 |
| gather_dep | cpu | 16 | 4 MiB | 95.6 | 1.34 | 5.619 | - | line_bytes=128 n_items=4194304 steps_per_thread=262144 |
| gather_dep | cpu | 17 | 4 MiB | 96.7 | 1.32 | 5.554 | - | line_bytes=128 n_items=4194304 steps_per_thread=246723 |
| gather_dep | cpu | 18 | 4 MiB | 98.8 | 1.30 | 5.436 | - | line_bytes=128 n_items=4194304 steps_per_thread=233016 |
| hashbrown_build | cpu | 1 | 4 MiB | 11.3 | 2.82 | 0.370 | - | key_bytes=8 n_items=131072 n_keys=131072 prealloc=true val_bytes=4 |
| hashbrown_probe | cpu | 12 | 4 MiB | 382.8 | 0.33 | 0.044 | - | hit_rate=0.500 key_bytes=8 n_items=131072 n_keys=131072 val_bytes=4 |
| hashbrown_probe | cpu | 16 | 4 MiB | 7.7 | 16.57 | 2.171 | - | hit_rate=0.500 key_bytes=8 n_items=131072 n_keys=131072 val_bytes=4 |
| hashbrown_probe | cpu | 17 | 4 MiB | 7.8 | 16.47 | 2.158 | - | hit_rate=0.500 key_bytes=8 n_items=131072 n_keys=131072 val_bytes=4 |
| hashbrown_probe | cpu | 18 | 4 MiB | 7.8 | 16.46 | 2.157 | - | hit_rate=0.500 key_bytes=8 n_items=131072 n_keys=131072 val_bytes=4 |
| sort_u32 | cpu | 12 | 4 MiB | 6.2 | 1.29 | 1.352 | - | algo=chunk sort_unstable + merge-path tree elem_bytes=4 includes_input_copy=true n_items=1048576 parallel=true |
| sort_u32 | cpu | 16 | 4 MiB | 0.7 | 11.81 | 12.389 | - | algo=chunk sort_unstable + merge-path tree elem_bytes=4 includes_input_copy=true n_items=1048576 parallel=true |
| sort_u32 | cpu | 17 | 4 MiB | 0.6 | 13.91 | 14.589 | - | algo=chunk sort_unstable + merge-path tree elem_bytes=4 includes_input_copy=true n_items=1048576 parallel=true |
| sort_u32 | cpu | 18 | 4 MiB | 0.6 | 13.76 | 14.433 | - | algo=chunk sort_unstable + merge-path tree elem_bytes=4 includes_input_copy=true n_items=1048576 parallel=true |
| merge_u32 | cpu | 12 | 4 MiB | 80.4 | 0.15 | 0.157 | - | elem_bytes=4 n_items=1048576 split=merge-path |
| merge_u32 | cpu | 16 | 4 MiB | 5.4 | 2.20 | 2.312 | - | elem_bytes=4 n_items=1048576 split=merge-path |
| merge_u32 | cpu | 17 | 4 MiB | 5.4 | 2.21 | 2.317 | - | elem_bytes=4 n_items=1048576 split=merge-path |
| merge_u32 | cpu | 18 | 4 MiB | 5.5 | 2.19 | 2.299 | - | elem_bytes=4 n_items=1048576 split=merge-path |
| hist_scan | cpu | 12 | 4 MiB | 27.8 | 0.14 | 0.151 | - | bins=65536 elem_bytes=4 n_items=1048576 |
| hist_scan | cpu | 16 | 4 MiB | 1.0 | 4.17 | 4.369 | - | bins=65536 elem_bytes=4 n_items=1048576 |
| hist_scan | cpu | 17 | 4 MiB | 1.0 | 4.16 | 4.360 | - | bins=65536 elem_bytes=4 n_items=1048576 |
| hist_scan | cpu | 18 | 4 MiB | 1.0 | 4.16 | 4.360 | - | bins=65536 elem_bytes=4 n_items=1048576 |
