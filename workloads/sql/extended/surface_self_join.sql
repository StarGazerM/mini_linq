-- Two edges share their inner endpoint; increment the final endpoint.
SELECT DISTINCT first.c0, second.c1 + 1
FROM "mini-linq/raw/0" AS first
JOIN "mini-linq/raw/0" AS second ON first.c1 = second.c0
WHERE first.c0 < second.c1 ORDER BY 1, 2;
