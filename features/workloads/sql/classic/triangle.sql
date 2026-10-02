-- Close a directed three-edge colored cycle.
SELECT DISTINCT r.c0, r.c1, s.c1
FROM "mini-linq/raw/0" AS r
JOIN "mini-linq/raw/1" AS s ON s.c0 = r.c1
JOIN "mini-linq/raw/2" AS t ON t.c0 = s.c1 AND t.c1 = r.c0
ORDER BY 1, 2, 3;
