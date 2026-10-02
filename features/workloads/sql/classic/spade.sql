-- A triangle with a handle from its first vertex; T is oriented x to z.
SELECT DISTINCT r.c0, r.c1, s.c1, handle.c1
FROM "mini-linq/raw/0" AS r
JOIN "mini-linq/raw/1" AS s ON s.c0 = r.c1
JOIN "mini-linq/raw/2" AS t ON t.c0 = r.c0 AND t.c1 = s.c1
JOIN "mini-linq/raw/3" AS handle ON handle.c0 = r.c0
ORDER BY 1, 2, 3, 4;
