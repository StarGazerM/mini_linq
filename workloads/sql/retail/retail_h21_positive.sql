-- A positive same-order witness; the two suppliers may be equal.
SELECT DISTINCT waiting.c0, supplier.c0, other.c2
FROM "mini-linq/raw/0" AS supplier
JOIN "mini-linq/raw/1" AS waiting ON waiting.c2 = supplier.c0
JOIN "mini-linq/raw/2" AS orders ON orders.c0 = waiting.c0
JOIN "mini-linq/raw/3" AS nation ON nation.c0 = supplier.c1
JOIN "mini-linq/raw/1" AS other ON other.c0 = waiting.c0
ORDER BY 1, 2, 3;
