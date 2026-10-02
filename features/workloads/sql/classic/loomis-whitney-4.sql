-- Four ternary projections agree on every shared coordinate.
SELECT DISTINCT abc.c0, abc.c1, abc.c2, bcd.c2
FROM "mini-linq/raw/0" AS abc
JOIN "mini-linq/raw/1" AS bcd ON bcd.c0 = abc.c1 AND bcd.c1 = abc.c2
JOIN "mini-linq/raw/2" AS acd ON acd.c0 = abc.c0 AND acd.c1 = abc.c2 AND acd.c2 = bcd.c2
JOIN "mini-linq/raw/3" AS abd ON abd.c0 = abc.c0 AND abd.c1 = abc.c1 AND abd.c2 = bcd.c2
ORDER BY 1, 2, 3, 4;
