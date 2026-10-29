-- Preserve disjunction grouping and relational inequality (NOT equality).
SELECT DISTINCT c0, c1 FROM "mini-linq/raw/0"
WHERE (c0 < 0 OR c1 > 2) AND NOT (c0 = c1) ORDER BY 1, 2;
