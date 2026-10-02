-- Close a directed four-edge colored cycle.
SELECT DISTINCT r.c0, r.c1, s.c1, t.c1
FROM "mini-linq/raw/0" AS r
JOIN "mini-linq/raw/1" AS s ON s.c0 = r.c1
JOIN "mini-linq/raw/2" AS t ON t.c0 = s.c1
JOIN "mini-linq/raw/3" AS u ON u.c0 = t.c1 AND u.c1 = r.c0
ORDER BY 1, 2, 3, 4;
