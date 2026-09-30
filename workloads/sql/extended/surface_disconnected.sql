-- Independent atoms meet only in the later arithmetic and comparison.
SELECT DISTINCT r.c0 - s.c0
FROM "mini-linq/raw/0" AS r CROSS JOIN "mini-linq/raw/1" AS s
WHERE r.c0 > s.c0 ORDER BY 1;
