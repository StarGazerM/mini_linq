import unittest

from expansion_report import summarize


class ExpansionReportTests(unittest.TestCase):
    def test_stage_totals_and_repeated_admission(self):
        rows = []
        for iteration in [0, 1]:
            for definition, children in [("first", 13), ("last", 7)]:
                common = f"rule=R\tvariant=2\titeration={iteration}\tprogram=P\tdefinition={definition}"
                rows.append(f"cq_expansion\t{common}\tstage=0\tchildren={children}\n")
                rows.append(f"cq_expansion_output\t{common}\toutput=5\tadmitted=3\n")
        result, = summarize(rows)
        self.assertEqual(result["admitted"], 6)
        self.assertEqual(result["largest_preclosing"], 26)
        self.assertEqual(result["largest_preclosing_per_output"], 26 / 6)
        self.assertEqual([stage["children"] for stage in result["stages"]], [26, 14])

    def test_terminal_output_is_not_the_largest_intermediate(self):
        common = "rule=R\tvariant=0\titeration=0\tprogram=P"
        result, = summarize([
            f"cq_expansion\t{common}\tdefinition=first\tstage=0\tchildren=100\n",
            f"cq_expansion_output\t{common}\tdefinition=first\toutput=100\tadmitted=100\n",
            f"cq_expansion_output\t{common}\tdefinition=last\toutput=2\tadmitted=2\n",
        ])
        self.assertEqual(result["admitted"], 2)
        self.assertEqual(result["largest_preclosing_per_output"], 50)

    def test_empty_output_is_not_infinite_json(self):
        result, = summarize([
            "unrelated diagnostic\n",
            "cq_expansion\trule=R\tvariant=0\titeration=0\tprogram=P\tdefinition=d\tstage=0\tchildren=12\n",
            "cq_expansion_output\trule=R\tvariant=0\titeration=0\tprogram=P\tdefinition=d\toutput=0\tadmitted=0\n",
        ])
        self.assertIsNone(result["largest_preclosing_per_output"])


if __name__ == "__main__":
    unittest.main()
