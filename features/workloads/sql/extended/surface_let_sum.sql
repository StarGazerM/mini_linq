-- The computed total is both filtered and returned.
WITH totals AS (SELECT c0 AS x, c0 + c1 AS total FROM "mini-linq/raw/0")
SELECT DISTINCT x, total FROM totals WHERE total > 0 ORDER BY 1, 2;
