-- Each A value paired with each B pair, with no linking key.
SELECT DISTINCT a.c0, b.c0, b.c1
FROM "mini-linq/raw/0" AS a CROSS JOIN "mini-linq/raw/1" AS b
ORDER BY 1, 2, 3;
