-- Projection computes two expressions before removing duplicates.
SELECT DISTINCT c0 - c1, c0 * 2 + c1 FROM "mini-linq/raw/0"
WHERE c0 >= 0 ORDER BY 1, 2;
