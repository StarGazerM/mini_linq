-- Two signed integer comparisons constrain the original row.
SELECT DISTINCT c0, c1 FROM "mini-linq/raw/0"
WHERE c0 < c1 AND c0 >= -2 ORDER BY 1, 2;
