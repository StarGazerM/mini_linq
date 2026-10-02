-- Customer and supplier must belong to the same nation, with a valid region.
SELECT DISTINCT orders.c0, supplier.c0, nation.c0
FROM "mini-linq/raw/1" AS orders
JOIN "mini-linq/raw/0" AS customer ON customer.c0 = orders.c1
JOIN "mini-linq/raw/2" AS lineitem ON lineitem.c0 = orders.c0
JOIN "mini-linq/raw/3" AS supplier ON supplier.c0 = lineitem.c2 AND supplier.c1 = customer.c1
JOIN "mini-linq/raw/4" AS nation ON nation.c0 = customer.c1
JOIN "mini-linq/raw/5" AS region ON region.c0 = nation.c1
ORDER BY 1, 2, 3;
