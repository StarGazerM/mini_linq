-- Three differently colored edges with the same center.
SELECT DISTINCT a.c0, a.c1, b.c1, c.c1
FROM "mini-linq/raw/0" AS a
JOIN "mini-linq/raw/1" AS b ON b.c0 = a.c0
JOIN "mini-linq/raw/2" AS c ON c.c0 = a.c0
ORDER BY 1, 2, 3, 4;
