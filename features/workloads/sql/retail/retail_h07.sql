-- Shipping joins retain independent supplier and customer nation roles.
SELECT DISTINCT orders.c0, supplier.c1, customer.c1
FROM "mini-linq/raw/1" AS lineitem
JOIN "mini-linq/raw/0" AS supplier ON supplier.c0 = lineitem.c2
JOIN "mini-linq/raw/2" AS orders ON orders.c0 = lineitem.c0
JOIN "mini-linq/raw/3" AS customer ON customer.c0 = orders.c1
WHERE EXISTS (SELECT 1 FROM "mini-linq/raw/4" AS nation WHERE nation.c0 = supplier.c1)
  AND EXISTS (SELECT 1 FROM "mini-linq/raw/4" AS nation WHERE nation.c0 = customer.c1)
ORDER BY 1, 2, 3;
