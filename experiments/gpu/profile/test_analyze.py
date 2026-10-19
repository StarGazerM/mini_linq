import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('analyze', Path(__file__).with_name('analyze.py'))
a = importlib.util.module_from_spec(spec)
spec.loader.exec_module(a)


class Intervals(unittest.TestCase):
    def test_partition_intersection(self):
        self.assertEqual(a.intersection_duration([(0, 10), (20, 30)], [(5, 25)]), 10)

    def test_versioned_sync_names_and_transfer_direction(self):
        self.assertTrue(a.is_sync({'name': 'cudaStreamSynchronize_v3020'}, set()))
        self.assertTrue(a.is_sync({'name': 'cuMemcpyDtoHAsync_v2'}, set()))
        self.assertTrue(a.is_sync({'name': 'cudaMemcpyAsync_v3020', 'correlationId': 7}, {7}))
        self.assertFalse(a.is_sync({'name': 'cudaMemcpyAsync_v3020', 'correlationId': 8}, {7}))
        self.assertFalse(a.is_sync({'name': 'cudaLaunchKernel_v7000'}, set()))

    def test_union_clipping_and_idle(self):
        events = [{'start': 0, 'end': 10}, {'start': 5, 'end': 15}, {'start': 20, 'end': 22}]
        self.assertEqual(a.duration(a.clipped(events, [(3, 25)])), 14)
        self.assertEqual(a.gaps(events, [(3, 25)]), [5, 3])

    def test_stream_concurrency_counts_streams_not_kernels(self):
        kernels = [{'start': 0, 'end': 10, 'streamId': 1},
                   {'start': 5, 'end': 15, 'streamId': 2},
                   {'start': 5, 'end': 10, 'streamId': 1}]
        result = a.concurrency(kernels, [(0, 20)])
        self.assertEqual(result['max_streams'], 2)
        self.assertEqual(result['avg_streams_window'], 1)
        self.assertEqual(result['kernel_overlap_pct'], 25)
        self.assertAlmostEqual(result['avg_streams_kernel_busy'], 4 / 3)

    def test_disjoint_windows_do_not_count_inter_window_gaps(self):
        events = [{'start': 0, 'end': 100}]
        self.assertEqual(a.duration(a.clipped(events, [(0, 10), (90, 100)])), 20)
        self.assertEqual(a.gaps(events, [(0, 10), (90, 100)]), [])


if __name__ == '__main__':
    unittest.main()
