-- Attach one dimension value to each of the fact's three keys.
SELECT DISTINCT f.c0, f.c1, f.c2, da.c1, db.c1, dc.c1
FROM "mini-linq/raw/0" AS f
JOIN "mini-linq/raw/1" AS da ON da.c0 = f.c0
JOIN "mini-linq/raw/2" AS db ON db.c0 = f.c1
JOIN "mini-linq/raw/3" AS dc ON dc.c0 = f.c2
ORDER BY 1, 2, 3, 4, 5, 6;
