-- Inventory and sale dates share a week; ship date needs only a date witness.
SELECT DISTINCT sale.c0, sale.c1, inventory.c2, sold_date.c1
FROM "mini-linq/raw/0" AS sale
JOIN "mini-linq/raw/1" AS sold_date ON sold_date.c0 = sale.c4
JOIN "mini-linq/raw/2" AS inventory ON inventory.c0 = sale.c1
JOIN "mini-linq/raw/1" AS inventory_date ON inventory_date.c0 = inventory.c1 AND inventory_date.c1 = sold_date.c1
JOIN "mini-linq/raw/3" AS warehouse ON warehouse.c0 = inventory.c2
JOIN "mini-linq/raw/4" AS item ON item.c0 = sale.c1
JOIN "mini-linq/raw/5" AS customer_demo ON customer_demo.c0 = sale.c2
JOIN "mini-linq/raw/6" AS household_demo ON household_demo.c0 = sale.c3
WHERE EXISTS (SELECT 1 FROM "mini-linq/raw/1" AS ship_date WHERE ship_date.c0 = sale.c5)
ORDER BY 1, 2, 3, 4;
