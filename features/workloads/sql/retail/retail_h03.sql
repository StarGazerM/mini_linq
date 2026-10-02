-- Customer orders that have at least one lineitem.
SELECT DISTINCT orders.c0, customer.c0
FROM "mini-linq/raw/1" AS orders
JOIN "mini-linq/raw/0" AS customer ON customer.c0 = orders.c1
WHERE EXISTS (SELECT 1 FROM "mini-linq/raw/2" AS lineitem WHERE lineitem.c0 = orders.c0)
ORDER BY 1, 2;
