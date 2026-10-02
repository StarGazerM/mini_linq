-- Part, supplier nation, and customer region are independent branches.
SELECT DISTINCT lineitem.c0, lineitem.c1, lineitem.c2
FROM "mini-linq/raw/0" AS lineitem
JOIN "mini-linq/raw/1" AS part ON part.c0 = lineitem.c1
JOIN "mini-linq/raw/2" AS supplier ON supplier.c0 = lineitem.c2
JOIN "mini-linq/raw/3" AS orders ON orders.c0 = lineitem.c0
JOIN "mini-linq/raw/4" AS customer ON customer.c0 = orders.c1
JOIN "mini-linq/raw/5" AS customer_nation ON customer_nation.c0 = customer.c1
JOIN "mini-linq/raw/6" AS customer_region ON customer_region.c0 = customer_nation.c1
WHERE EXISTS (SELECT 1 FROM "mini-linq/raw/5" AS supplier_nation WHERE supplier_nation.c0 = supplier.c1)
ORDER BY 1, 2, 3;
