-- The last edge points into the second edge's destination.
SELECT DISTINCT first.c0, last.c0
FROM "mini-linq/raw/0" AS first
JOIN "mini-linq/raw/0" AS middle ON middle.c0 = first.c1
JOIN "mini-linq/raw/0" AS last ON last.c1 = middle.c1
ORDER BY 1, 2;
