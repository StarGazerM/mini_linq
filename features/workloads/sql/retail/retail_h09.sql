-- PartSupp validates the pair as a composite key.
SELECT DISTINCT lineitem.c0, lineitem.c1, lineitem.c2
FROM "mini-linq/raw/0" AS lineitem
JOIN "mini-linq/raw/1" AS partsupp ON partsupp.c0 = lineitem.c1 AND partsupp.c1 = lineitem.c2
JOIN "mini-linq/raw/2" AS part ON part.c0 = lineitem.c1
JOIN "mini-linq/raw/3" AS supplier ON supplier.c0 = lineitem.c2
JOIN "mini-linq/raw/4" AS orders ON orders.c0 = lineitem.c0
JOIN "mini-linq/raw/5" AS nation ON nation.c0 = supplier.c1
ORDER BY 1, 2, 3;
