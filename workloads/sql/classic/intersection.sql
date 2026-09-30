-- Values present in both input sets.
SELECT DISTINCT l.c0
FROM "mini-linq/raw/0" AS l
INNER JOIN "mini-linq/raw/1" AS r ON r.c0 = l.c0
ORDER BY 1;
