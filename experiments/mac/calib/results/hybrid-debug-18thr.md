| op | proc | thr | footprint | GB/s | ns/item | median ms | gpu ms | notes |
|---|---|---:|---:|---:|---:|---:|---:|---|
| seq_read | cpu | 18 | 1 GiB | 101.2 | 0.08 | 10.610 | - | elem_bytes=8 n_items=134217728 partner=gpu:seq_read:1073741824 partner_threads=1 slowdown=1.676 solo_gbps=169.644 solo_median_ms=6.329 unroll=8 |
| seq_read | gpu | 1 | 1 GiB | 183.0 | 0.02 | 5.866 | 5.552 | partner=cpu:seq_read:1073741824 partner_threads=18 slowdown=1.480 solo_gbps=270.967 solo_median_ms=3.963 |
| gather_line | cpu | 18 | 1 GiB | 75.5 | 1.70 | 14.225 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=8388608 partner=gpu:seq_copy:1073741824 partner_threads=1 slowdown=1.660 solo_gbps=125.341 solo_median_ms=8.567 |
| seq_copy | gpu | 1 | 1 GiB | 174.5 | 0.05 | 12.303 | 11.774 | partner=cpu:gather_line:1073741824 partner_threads=18 slowdown=1.475 solo_gbps=257.460 solo_median_ms=8.341 |
| gather_line | cpu | 18 | 64 MiB | 25.7 | 4.99 | 2.616 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=524288 partner=gpu:seq_copy:1073741824 partner_threads=1 slowdown=1.098 solo_gbps=28.180 solo_median_ms=2.381 |
| seq_copy | gpu | 1 | 1 GiB | 235.7 | 0.03 | 9.111 | 8.783 | partner=cpu:gather_line:67108864 partner_threads=18 slowdown=1.053 solo_gbps=248.268 solo_median_ms=8.650 |
| hashbrown_probe | cpu | 18 | 64 MiB | 49.6 | 2.58 | 5.413 | - | hit_rate=0.500 key_bytes=8 n_items=2097152 n_keys=2097152 partner=gpu:partition_pass:1073741824 partner_threads=1 slowdown=1.284 solo_gbps=63.677 solo_median_ms=4.216 val_bytes=4 |
| partition_pass | gpu | 1 | 1 GiB | 146.3 | 0.13 | 33.951 | 33.378 | correct=true partner=cpu:hashbrown_probe:67108864 partner_threads=18 radix_bits=8 slowdown=1.135 solo_gbps=166.089 solo_median_ms=29.900 stable=false tiles=131072 |
| hashbrown_probe | cpu | 18 | 1 GiB | 52.5 | 2.44 | 40.879 | - | hit_rate=0.500 key_bytes=8 n_items=16777216 n_keys=33554432 partner=gpu:partition_pass:1073741824 partner_threads=1 slowdown=1.592 solo_gbps=83.635 solo_median_ms=25.677 val_bytes=4 |
| partition_pass | gpu | 1 | 1 GiB | 126.1 | 0.15 | 39.387 | 39.106 | correct=true partner=cpu:hashbrown_probe:1073741824 partner_threads=18 radix_bits=8 slowdown=1.291 solo_gbps=162.714 solo_median_ms=30.520 stable=false tiles=131072 |
| seq_read | cpu | 6 | 1 GiB | 174.1 | 0.05 | 6.166 | - | elem_bytes=8 n_items=134217728 partner=gpu:gather_line:1073741824 partner_threads=1 slowdown=1.101 solo_gbps=191.681 solo_median_ms=5.602 unroll=8 |
| gather_line | gpu | 1 | 1 GiB | 184.8 | 0.69 | 23.243 | 22.989 | partner=cpu:seq_read:1073741824 partner_threads=6 slowdown=1.007 solo_gbps=186.112 solo_median_ms=23.077 |
| gather_line | cpu | 6 | 1 GiB | 100.0 | 1.28 | 10.741 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=8388608 partner=gpu:gather_line:1073741824 partner_threads=1 slowdown=1.115 solo_gbps=111.511 solo_median_ms=9.629 |
| gather_line | gpu | 1 | 1 GiB | 184.7 | 0.69 | 23.253 | 23.037 | partner=cpu:gather_line:1073741824 partner_threads=6 slowdown=1.012 solo_gbps=186.931 solo_median_ms=22.976 |
